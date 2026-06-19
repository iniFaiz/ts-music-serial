// ---------------------------------------------------------------------------
// Lyrics providers + LRC parsing
//
// All network access lives here (in Rust) rather than in the webview, so the
// app's strict CSP stays intact. The orchestration / caching / local-tag lookup
// lives in lib.rs (which owns the asset-protocol scope check); this module
// provides the pure LRC parser and the remote provider fetchers.
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

// One karaoke word/syllable with its own absolute start + duration (ms). Present
// only for providers that expose word-level timing (e.g. NetEase `yrc`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LyricWord {
    pub time_ms: u64,
    pub duration_ms: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LyricLine {
    // Milliseconds from the start of the track. None for unsynced (plain) lyrics.
    pub time_ms: Option<u64>,
    pub text: String,
    // Per-word timing for karaoke (word-by-word) rendering, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LyricWord>>,
    // Romanization (e.g. romaji) of this line, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romaji: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lyrics {
    pub synced: bool,
    pub source: String,
    pub lines: Vec<LyricLine>,
    // True when at least one line carries word-level (karaoke) timing.
    #[serde(default)]
    pub word_synced: bool,
    // True when at least one line carries a romanization.
    #[serde(default)]
    pub has_romaji: bool,
}

// Parse a single LRC time tag body ("mm:ss.xx" / "mm:ss") into milliseconds.
// Returns None for metadata tags like "ar:Artist" (non-numeric minute field).
fn parse_lrc_time(tag: &str) -> Option<u64> {
    let (min_part, sec_part) = tag.split_once(':')?;
    let minutes: u64 = min_part.trim().parse().ok()?;
    let seconds: f64 = sec_part.trim().parse().ok()?;
    if !(0.0..3600.0).contains(&seconds) {
        return None;
    }
    Some(minutes * 60_000 + (seconds * 1000.0) as u64)
}

// Parse LRC text into time-stamped lines. A line may carry multiple time tags
// (repeated chorus); each becomes its own entry. Lines without any time tag are
// skipped here (the caller falls back to plain rendering when none are found).
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw;
        let mut times: Vec<u64> = Vec::new();
        loop {
            rest = rest.trim_start();
            if !rest.starts_with('[') {
                break;
            }
            let close = match rest.find(']') {
                Some(i) => i,
                None => break,
            };
            let tag = &rest[1..close];
            if let Some(ms) = parse_lrc_time(tag) {
                times.push(ms);
                rest = &rest[close + 1..];
            } else if tag.contains(':') {
                // Metadata tag ([ar:], [ti:], [offset:], ...) — drop and continue.
                rest = &rest[close + 1..];
            } else {
                break;
            }
        }
        if times.is_empty() {
            continue;
        }
        let content = rest.trim().to_string();
        for ms in times {
            lines.push(LyricLine {
                time_ms: Some(ms),
                text: content.clone(),
                words: None,
                romaji: None,
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms.unwrap_or(0));
    lines
}

// Treat raw text as unsynced lyrics, one entry per line.
pub fn make_plain(text: &str) -> Vec<LyricLine> {
    text.lines()
        .map(|l| LyricLine {
            time_ms: None,
            text: l.trim_end().to_string(),
            words: None,
            romaji: None,
        })
        .collect()
}

// Build a Lyrics from arbitrary text: synced when it contains LRC timestamps,
// otherwise plain. None when there's no usable content.
pub fn lyrics_from_text(text: &str, source: &str) -> Option<Lyrics> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let synced = parse_lrc(t);
    if !synced.is_empty() {
        return Some(Lyrics {
            synced: true,
            source: source.to_string(),
            lines: synced,
            word_synced: false,
            has_romaji: false,
        });
    }
    let plain = make_plain(t);
    if plain.iter().any(|l| !l.text.trim().is_empty()) {
        Some(Lyrics {
            synced: false,
            source: source.to_string(),
            lines: plain,
            word_synced: false,
            has_romaji: false,
        })
    } else {
        None
    }
}

// ---- Provider: LRCLIB -----------------------------------------------------

fn lrclib_value_to_lyrics(v: &serde_json::Value) -> Option<Lyrics> {
    if v.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false) {
        return None;
    }
    if let Some(s) = v.get("syncedLyrics").and_then(|x| x.as_str()) {
        if let Some(l) = lyrics_from_text(s, "LRCLIB") {
            return Some(l);
        }
    }
    if let Some(s) = v.get("plainLyrics").and_then(|x| x.as_str()) {
        if let Some(l) = lyrics_from_text(s, "LRCLIB") {
            return Some(l);
        }
    }
    None
}

pub async fn from_lrclib(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    album: &str,
    duration: u64,
) -> Option<Lyrics> {
    let dur = duration.to_string();
    // Targeted lookup first.
    if let Ok(resp) = client
        .get("https://lrclib.net/api/get")
        .query(&[
            ("track_name", title),
            ("artist_name", artist),
            ("album_name", album),
            ("duration", dur.as_str()),
        ])
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                if let Some(l) = lrclib_value_to_lyrics(&v) {
                    return Some(l);
                }
            }
        }
    }
    // Fuzzy search fallback.
    if let Ok(resp) = client
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", title), ("artist_name", artist)])
        .send()
        .await
    {
        if let Ok(v) = resp.json::<serde_json::Value>().await {
            if let Some(items) = v.as_array() {
                for it in items {
                    if let Some(l) = lrclib_value_to_lyrics(it) {
                        return Some(l);
                    }
                }
            }
        }
    }
    None
}

// ---- Provider: NetEase Cloud Music ----------------------------------------

pub fn is_netease_metadata(line: &LyricLine) -> bool {
    let t = line.text.trim();
    if t.is_empty() {
        return false;
    }

    // Only filter out lines at the very beginning of the song (first 60 seconds)
    // for synced lyrics, or all lines if unsynced.
    if let Some(ms) = line.time_ms {
        if ms >= 60_000 {
            return false;
        }
    }

    let t_lower = t.to_lowercase();
    if t_lower.starts_with("by:") 
        || t_lower.starts_with("lrc:") 
        || t_lower.starts_with("translator:") 
        || t_lower.starts_with("lyrics by") 
        || t_lower.starts_with("composed by") 
        || t_lower.starts_with("produced by")
        || t_lower.starts_with("arranged by")
        || t_lower.starts_with("remix by")
        || t_lower.starts_with("vocals by")
        || t_lower.starts_with("instrumental by")
    {
        return true;
    }

    // Chinese metadata keywords. If they contain a separator (colon, slash, space, hyphen, etc.)
    // or if the line starts with them and is short.
    let separators = &[':', '：', '/', '-', '—', '|', ' '];
    let keywords = &[
        "作词", "作曲", "编曲", "制作人", "监制", "和声", "混音", "母带", 
        "录音", "吉他", "贝斯", "鼓", "键盘", "策划", "宣发", "发卡", "设计",
        "原唱", "翻唱", "伴奏", "后期", "音响", "企划", "统筹", "出品", "发行"
    ];

    for &k in keywords {
        if t.contains(k) {
            let idx = t.find(k).unwrap();
            let after_k = &t[idx + k.len()..];
            let after_trimmed = after_k.trim_start();
            if after_trimmed.starts_with(|c| separators.contains(&c)) || after_k.starts_with(' ') || after_k.is_empty() {
                return true;
            }
        }
    }

    // Highly specific terms (watermarks or contributor labels) that don't need separators to be safe
    let specific_keywords = &[
        "歌词贡献", "翻译贡献", "贡献者", "歌词及翻译", "网易云", "网易首发",
        "时间轴", "和声编写", "歌词制作", "制作歌词", "歌词由", "本歌词由", 
        "此歌词由", "感谢您的支持", "感谢您支持", "QQ音乐", "腾讯音乐", 
        "酷狗", "酷我", "虾米音乐", "提供", "上传", "校对", "同步", 
        "有疑问请联系", "lrc制作", "lrc下载", "歌词下载"
    ];
    for &k in specific_keywords {
        if t.contains(k) {
            return true;
        }
    }

    false
}

const NETEASE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// Parse NetEase `yrc` (verbatim/word-by-word) lyrics into word-timed lines.
//
// Each line is `[lineStartMs,lineDurationMs]` followed by word tokens
// `(wordStartMs,wordDurationMs,0)wordText`, where the word start is absolute ms
// from the track start. Leading JSON metadata lines (`{...}`) are skipped.
fn parse_yrc(text: &str) -> Vec<LyricLine> {
    let mut out: Vec<LyricLine> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        // Skip blanks and the JSON metadata header lines yrc sometimes carries.
        if line.is_empty() || !line.starts_with('[') {
            continue;
        }
        let close = match line.find(']') {
            Some(i) => i,
            None => continue,
        };
        let line_start = match line[1..close]
            .split(',')
            .next()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(v) => v,
            None => continue,
        };

        let mut rest = &line[close + 1..];
        let mut words: Vec<LyricWord> = Vec::new();
        let mut full = String::new();
        // Walk each `(start,dur,0)text` token. `(`/`)` are ASCII, so all the
        // byte-index slicing below lands on valid UTF-8 boundaries.
        while let Some(op) = rest.find('(') {
            let cl = match rest[op..].find(')') {
                Some(i) => op + i,
                None => break,
            };
            let mut meta = rest[op + 1..cl].split(',');
            let w_start = meta.next().and_then(|s| s.trim().parse::<u64>().ok());
            let w_dur = meta.next().and_then(|s| s.trim().parse::<u64>().ok());
            // Word text runs from after this `)` up to the next `(`.
            let after = &rest[cl + 1..];
            let next = after.find('(').unwrap_or(after.len());
            let word_text = &after[..next];
            if let (Some(ws), Some(wd)) = (w_start, w_dur) {
                if !word_text.is_empty() {
                    full.push_str(word_text);
                    words.push(LyricWord {
                        time_ms: ws,
                        duration_ms: wd,
                        text: word_text.to_string(),
                    });
                }
            }
            rest = &after[next..];
        }

        let text_join = full.trim().to_string();
        if words.is_empty() || text_join.is_empty() {
            continue;
        }
        out.push(LyricLine {
            time_ms: Some(line_start),
            text: text_join,
            words: Some(words),
            romaji: None,
        });
    }
    out.sort_by_key(|l| l.time_ms.unwrap_or(0));
    out
}

// Attach a romanization to each main line, pairing by index when the two line
// lists line up (the usual NetEase case), else by nearest timestamp. Romaji that
// merely repeats the original (Latin-script songs) is ignored. Returns whether
// any romaji was actually attached.
fn attach_romaji(main: &mut [LyricLine], romaji: &[LyricLine]) -> bool {
    if romaji.is_empty() {
        return false;
    }
    let mut attached = false;
    if romaji.len() == main.len() {
        for (m, r) in main.iter_mut().zip(romaji.iter()) {
            let rt = r.text.trim();
            if !rt.is_empty() && rt != m.text.trim() {
                m.romaji = Some(rt.to_string());
                attached = true;
            }
        }
    } else {
        for m in main.iter_mut() {
            let mt = match m.time_ms {
                Some(t) => t as i64,
                None => continue,
            };
            let mut best: Option<(i64, &str)> = None;
            for r in romaji {
                if let Some(rt_ms) = r.time_ms {
                    let d = (rt_ms as i64 - mt).abs();
                    if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                        best = Some((d, r.text.trim()));
                    }
                }
            }
            if let Some((d, rt)) = best {
                if d <= 1500 && !rt.is_empty() && rt != m.text.trim() {
                    m.romaji = Some(rt.to_string());
                    attached = true;
                }
            }
        }
    }
    attached
}

pub async fn from_netease(client: &reqwest::Client, title: &str, artist: &str) -> Option<Lyrics> {
    let q = format!("{title} {artist}");
    let search = client
        .get("https://music.163.com/api/search/get")
        .query(&[
            ("s", q.as_str()),
            ("type", "1"),
            ("limit", "5"),
            ("offset", "0"),
        ])
        .header("Referer", "https://music.163.com")
        .header("User-Agent", NETEASE_UA)
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = search.json().await.ok()?;
    let songs = v.get("result")?.get("songs")?.as_array()?;
    let id = songs.first()?.get("id")?.as_i64()?;
    let ids = id.to_string();

    // Prefer the encrypted eapi (the only endpoint that serves word-by-word
    // `yrc`); fall back to the plain v1 GET (line-level `lrc` + romaji), then the
    // classic endpoint (`lrc` only).
    if let Some(l) = netease_lyric_eapi(client, &ids).await {
        return Some(l);
    }
    if let Some(l) = netease_lyric_v1(client, &ids).await {
        return Some(l);
    }
    netease_lyric_classic(client, &ids).await
}

// v1 endpoint: returns lrc, romalrc (line romaji), yrc (word-by-word) and
// yromalrc (word-by-word romaji). Builds the richest Lyrics it can.
async fn netease_lyric_v1(client: &reqwest::Client, ids: &str) -> Option<Lyrics> {
    let resp = client
        .get("https://music.163.com/api/song/lyric/v1")
        .query(&[
            ("id", ids),
            ("cp", "false"),
            ("lv", "0"),
            ("kv", "0"),
            ("tv", "0"),
            ("rv", "0"),
            ("yv", "0"),
            ("ytv", "0"),
            ("yrv", "0"),
        ])
        .header("Referer", "https://music.163.com")
        .header("User-Agent", NETEASE_UA)
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    build_netease_lyrics(&v)
}

// Build the richest Lyrics from a NetEase lyric response (eapi or v1 GET):
// word-by-word `yrc` when present (else line-level `lrc`), plus a romaji
// sub-line from `yromalrc`/`romalrc` when available.
fn build_netease_lyrics(v: &serde_json::Value) -> Option<Lyrics> {
    let field = |k: &str| {
        v.get(k)
            .and_then(|x| x.get("lyric"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    };
    let lrc = field("lrc");
    let yrc = field("yrc");
    let romalrc = field("romalrc");
    let yromalrc = field("yromalrc");

    // Main lines: prefer word-by-word yrc, else line-level lrc.
    let (mut lines, word_synced) = match yrc.as_ref().map(|y| parse_yrc(y)) {
        Some(p) if !p.is_empty() => (p, true),
        _ => (Vec::new(), false),
    };
    if lines.is_empty() {
        if let Some(l) = lrc.as_ref() {
            lines = parse_lrc(l);
        }
    }
    if lines.is_empty() {
        return None;
    }

    // Romaji: align with the form we used for the main lines.
    let romaji_lines: Option<Vec<LyricLine>> = if word_synced {
        yromalrc
            .as_ref()
            .map(|s| parse_yrc(s))
            .filter(|v| !v.is_empty())
            .or_else(|| romalrc.as_ref().map(|s| parse_lrc(s)).filter(|v| !v.is_empty()))
    } else {
        romalrc
            .as_ref()
            .map(|s| parse_lrc(s))
            .filter(|v| !v.is_empty())
            .or_else(|| yromalrc.as_ref().map(|s| parse_yrc(s)).filter(|v| !v.is_empty()))
    };
    let has_romaji = match romaji_lines.as_ref() {
        Some(r) => attach_romaji(&mut lines, r),
        None => false,
    };

    let mut lyrics = Lyrics {
        synced: true,
        source: "NetEase".to_string(),
        lines,
        word_synced,
        has_romaji,
    };
    lyrics.lines.retain(|line| !is_netease_metadata(line));
    if lyrics.lines.is_empty() {
        return None;
    }
    Some(lyrics)
}

// AES-128-ECB + MD5 request signing for NetEase's eapi. Produces the hex
// `params` body the eapi endpoints expect. Both the digest and the embedded
// payload use the same `json` string, so the server's digest check passes.
fn eapi_params(path: &str, json: &str) -> String {
    use aes::Aes128;
    use ecb::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyInit};
    use md5::{Digest, Md5};

    let message = format!("nobody{path}use{json}md5forencrypt");
    let digest = hex::encode(Md5::digest(message.as_bytes()));
    let data = format!("{path}-36cd479b6b5-{json}-36cd479b6b5-{digest}");

    let enc = ecb::Encryptor::<Aes128>::new_from_slice(b"e82ckenh8dichen8").unwrap();
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(data.as_bytes());
    hex::encode_upper(ct)
}

// eapi endpoint: the encrypted interface that (unlike the web API) returns
// word-by-word `yrc`/`yromalrc` for songs that have them.
async fn netease_lyric_eapi(client: &reqwest::Client, ids: &str) -> Option<Lyrics> {
    let path = "/api/song/lyric/v1";
    let json = format!(
        "{{\"id\":\"{ids}\",\"cp\":false,\"lv\":0,\"kv\":0,\"tv\":0,\"rv\":0,\"yv\":0,\"ytv\":0,\"yrv\":0,\"os\":\"pc\"}}"
    );
    let body = format!("params={}", eapi_params(path, &json));
    let resp = client
        .post("https://interface3.music.163.com/eapi/song/lyric/v1")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Referer", "https://music.163.com")
        .header("User-Agent", NETEASE_UA)
        .header("Cookie", "os=pc; appver=2.9.7")
        .body(body)
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    build_netease_lyrics(&v)
}

// Classic endpoint fallback: line-level lrc only (the previous behaviour).
async fn netease_lyric_classic(client: &reqwest::Client, ids: &str) -> Option<Lyrics> {
    let lyric = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[
            ("os", "pc"),
            ("id", ids),
            ("lv", "-1"),
            ("kv", "-1"),
            ("tv", "-1"),
        ])
        .header("Referer", "https://music.163.com")
        .header("User-Agent", NETEASE_UA)
        .send()
        .await
        .ok()?;
    let lv: serde_json::Value = lyric.json().await.ok()?;
    let lrc = lv.get("lrc")?.get("lyric")?.as_str()?;

    if let Some(mut lyrics) = lyrics_from_text(lrc, "NetEase") {
        lyrics.lines.retain(|line| !is_netease_metadata(line));
        Some(lyrics)
    } else {
        None
    }
}

// ---- Provider: Musixmatch (requires a community user token) ----------------

use std::sync::Mutex;

static CACHED_TOKEN: Mutex<Option<String>> = Mutex::new(None);
static CACHED_COOKIE: Mutex<Option<String>> = Mutex::new(None);

fn generate_token_guid() -> String {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(123456789);

    let mut seed = nanos;
    let mut next_random = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed
    };

    let parts = vec![
        format!("{:08x}", (next_random() as u32)),
        format!("{:04x}", ((next_random() >> 16) as u16)),
        format!("{:04x}", ((next_random() >> 16) as u16)),
        format!("{:04x}", ((next_random() >> 16) as u16)),
        format!("{:012x}", ((next_random() as u64) & 0xffffffffffffu64)),
    ];
    parts.join("-")
}

fn clean_song_title(title: &str) -> String {
    let mut cleaned = title.to_string();
    let patterns = &[
        "feat.", "featuring", "remaster", "single version", "album version", 
        "radio edit", "extended mix", "live version", "official video", 
        "official audio", "lyric video", "bonus track", "acoustic version",
        "original mix", "deluxe version", "special version", "with "
    ];

    if let Some(pos) = cleaned.rfind('(') {
        let inside = &cleaned[pos + 1..];
        let inside_lower = inside.to_lowercase();
        if patterns.iter().any(|&p| inside_lower.contains(p)) || inside_lower.contains("version") || inside_lower.contains("remix") {
            cleaned = cleaned[..pos].trim().to_string();
        }
    }
    if let Some(pos) = cleaned.rfind('[') {
        let inside = &cleaned[pos + 1..];
        let inside_lower = inside.to_lowercase();
        if patterns.iter().any(|&p| inside_lower.contains(p)) || inside_lower.contains("version") || inside_lower.contains("remix") {
            cleaned = cleaned[..pos].trim().to_string();
        }
    }
    if let Some(pos) = cleaned.rfind(" - ") {
        let after = &cleaned[pos + 3..];
        let after_lower = after.to_lowercase();
        if patterns.iter().any(|&p| after_lower.contains(p)) || after_lower.contains("version") || after_lower.contains("remix") || after_lower.contains("remaster") {
            cleaned = cleaned[..pos].trim().to_string();
        }
    }
    if cleaned.is_empty() {
        title.to_string()
    } else {
        cleaned
    }
}

pub async fn get_musixmatch_token(client: &reqwest::Client) -> Option<(String, String)> {
    let guid = generate_token_guid();
    let initial_cookie = format!("AWSELB=0; AWSELBCORS=0; x-mxm-token-guid={guid}");
    let resp = client
        .get("https://apic-desktop.musixmatch.com/ws/1.1/token.get")
        .query(&[
            ("format", "json"),
            ("app_id", "web-desktop-app-v1.0"),
        ])
        .header("Cookie", initial_cookie.as_str())
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Musixmatch/0.19.4 Chrome/58.0.3029.110 Electron/1.7.6 Safari/537.36")
        .send()
        .await
        .ok()?;

    let mut cookie_header = initial_cookie;
    for cookie in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        if let Ok(cookie_str) = cookie.to_str() {
            if let Some(cookie_val) = cookie_str.split(';').next() {
                cookie_header.push_str("; ");
                cookie_header.push_str(cookie_val.trim());
            }
        }
    }

    let v: serde_json::Value = resp.json().await.ok()?;
    let token = v
        .get("message")?
        .get("body")?
        .get("user_token")?
        .as_str()?;
    Some((token.to_string(), cookie_header))
}

async fn fetch_musixmatch_raw(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    album: &str,
    duration: u64,
    token: &str,
    cookie_header: &str,
) -> Option<serde_json::Value> {
    let dur = duration.to_string();
    let resp = client
        .get("https://apic-desktop.musixmatch.com/ws/1.1/macro.subtitles.get")
        .query(&[
            ("format", "json"),
            ("namespace", "lyrics_richsynched"),
            ("subtitle_format", "lrc"),
            ("app_id", "web-desktop-app-v1.0"),
            ("usertoken", token),
            ("q_track", title),
            ("q_artist", artist),
            ("q_album", album),
            ("q_duration", dur.as_str()),
            ("user_language", "en"),
        ])
        .header("Cookie", cookie_header)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Musixmatch/0.19.4 Chrome/58.0.3029.110 Electron/1.7.6 Safari/537.36")
        .send()
        .await
        .ok()?;
    resp.json().await.ok()
}

async fn fetch_musixmatch_fuzzy(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    token: &str,
    cookie_header: &str,
) -> Option<serde_json::Value> {
    let resp = client
        .get("https://apic-desktop.musixmatch.com/ws/1.1/macro.subtitles.get")
        .query(&[
            ("format", "json"),
            ("namespace", "lyrics_richsynched"),
            ("subtitle_format", "lrc"),
            ("app_id", "web-desktop-app-v1.0"),
            ("usertoken", token),
            ("q_track", title),
            ("q_artist", artist),
            ("user_language", "en"),
        ])
        .header("Cookie", cookie_header)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Musixmatch/0.19.4 Chrome/58.0.3029.110 Electron/1.7.6 Safari/537.36")
        .send()
        .await
        .ok()?;
    resp.json().await.ok()
}

// Parse Musixmatch "richsync" (word-by-word) into word-timed lines.
//
// `richsync_body` is a JSON-encoded string holding an array of lines, each:
// `{ "ts": lineStartSec, "te": lineEndSec, "x": "full text",
//    "l": [ { "c": "chunk", "o": offsetSecFromTs }, ... ] }`.
fn parse_musixmatch_richsync(body: &str) -> Vec<LyricLine> {
    let arr: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match arr.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut out: Vec<LyricLine> = Vec::new();
    for item in arr {
        let ts = item.get("ts").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let te = item.get("te").and_then(|x| x.as_f64()).unwrap_or(ts);
        let x = item.get("x").and_then(|x| x.as_str()).unwrap_or("");

        let mut words: Vec<LyricWord> = Vec::new();
        if let Some(chunks) = item.get("l").and_then(|x| x.as_array()) {
            for (i, ch) in chunks.iter().enumerate() {
                let c = ch.get("c").and_then(|x| x.as_str()).unwrap_or("");
                if c.is_empty() {
                    continue;
                }
                let o = ch.get("o").and_then(|x| x.as_f64()).unwrap_or(0.0);
                // The chunk ends where the next one starts (or at the line end).
                let next_o = chunks
                    .get(i + 1)
                    .and_then(|n| n.get("o"))
                    .and_then(|x| x.as_f64())
                    .unwrap_or(te - ts);
                let start_ms = ((ts + o) * 1000.0).max(0.0) as u64;
                let dur_ms = ((next_o - o) * 1000.0).max(0.0) as u64;
                words.push(LyricWord {
                    time_ms: start_ms,
                    duration_ms: dur_ms,
                    text: c.to_string(),
                });
            }
        }

        let line_text = if !x.trim().is_empty() {
            x.to_string()
        } else {
            words.iter().map(|w| w.text.as_str()).collect::<String>()
        };
        if words.is_empty() && line_text.trim().is_empty() {
            continue;
        }
        out.push(LyricLine {
            time_ms: Some((ts * 1000.0).max(0.0) as u64),
            text: line_text.trim().to_string(),
            words: if words.is_empty() { None } else { Some(words) },
            romaji: None,
        });
    }
    out.sort_by_key(|l| l.time_ms.unwrap_or(0));
    out
}

fn parse_musixmatch_value(v: &serde_json::Value) -> Option<Lyrics> {
    // 0. Prefer word-by-word richsync when present (the lyrics_richsynched
    //    namespace returns it for songs Musixmatch has aligned per word).
    if let Some(body) = v
        .get("message")
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("macro_calls"))
        .and_then(|mc| mc.get("track.richsync.get"))
        .and_then(|tr| tr.get("message"))
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("richsync"))
        .and_then(|r| r.get("richsync_body"))
        .and_then(|x| x.as_str())
    {
        let lines = parse_musixmatch_richsync(body);
        if !lines.is_empty() {
            return Some(Lyrics {
                synced: true,
                source: "Musixmatch".to_string(),
                lines,
                word_synced: true,
                has_romaji: false,
            });
        }
    }

    // 1. Try to extract synced subtitles first
    if let Some(subtitle) = v
        .get("message")
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("macro_calls"))
        .and_then(|mc| mc.get("track.subtitles.get"))
        .and_then(|ts| ts.get("message"))
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("subtitle_list"))
        .and_then(|sl| sl.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.get("subtitle"))
    {
        if let Some(body) = subtitle.get("subtitle_body").and_then(|x| x.as_str()) {
            if let Some(lyrics) = lyrics_from_text(body, "Musixmatch") {
                return Some(lyrics);
            }
        }
    }

    // 2. Fallback to unsynced lyrics if subtitles are not found
    if let Some(lyrics_obj) = v
        .get("message")
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("macro_calls"))
        .and_then(|mc| mc.get("track.lyrics.get"))
        .and_then(|tl| tl.get("message"))
        .and_then(|m| m.get("body"))
        .and_then(|b| b.get("lyrics"))
    {
        if let Some(body) = lyrics_obj.get("lyrics_body").and_then(|x| x.as_str()) {
            if let Some(lyrics) = lyrics_from_text(body, "Musixmatch") {
                return Some(lyrics);
            }
        }
    }

    None
}

pub async fn from_musixmatch(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    album: &str,
    duration: u64,
    token: &str,
) -> Option<Lyrics> {
    let mut current_token = token.to_string();
    let mut current_cookie = String::new();

    // 1. If no user token in settings, check memory cache
    if current_token.trim().is_empty() {
        if let Ok(guard) = CACHED_TOKEN.lock() {
            if let Some(cached) = &*guard {
                current_token = cached.clone();
            }
        }
        if let Ok(guard) = CACHED_COOKIE.lock() {
            if let Some(cached) = &*guard {
                current_cookie = cached.clone();
            }
        }
    }

    // 2. If token is still empty, fetch fresh one
    if current_token.trim().is_empty() {
        if let Some((t, cookie)) = get_musixmatch_token(client).await {
            current_token = t.clone();
            current_cookie = cookie.clone();

            // Save to memory cache
            if let Ok(mut guard) = CACHED_TOKEN.lock() {
                *guard = Some(t);
            }
            if let Ok(mut guard) = CACHED_COOKIE.lock() {
                *guard = Some(cookie);
            }
        } else {
            return None;
        }
    }

    // 3. Ensure we have basic cookie headers if none was cached
    if current_cookie.is_empty() {
        current_cookie = format!("AWSELB=0; AWSELBCORS=0; x-mxm-token-guid={}", generate_token_guid());
    }

    let mut v = fetch_musixmatch_raw(client, title, artist, album, duration, &current_token, &current_cookie).await?;

    // Check for 401 / renew error in response
    let mut status_code = v
        .get("message")
        .and_then(|m| m.get("header"))
        .and_then(|h| h.get("status_code"))
        .and_then(|s| s.as_i64());

    if status_code == Some(401) {
        // Token is invalid/expired. Retrieve a fresh one.
        if let Some((t, cookie)) = get_musixmatch_token(client).await {
            current_token = t.clone();
            current_cookie = cookie.clone();

            // Update memory cache
            if let Ok(mut guard) = CACHED_TOKEN.lock() {
                *guard = Some(t);
            }
            if let Ok(mut guard) = CACHED_COOKIE.lock() {
                *guard = Some(cookie);
            }

            // Retry with new token and sticky cookies
            v = fetch_musixmatch_raw(client, title, artist, album, duration, &current_token, &current_cookie).await?;
        }
    }

    // Try parsing strict response first
    if let Some(lyrics) = parse_musixmatch_value(&v) {
        return Some(lyrics);
    }

    // Fallback: retry with fuzzy parameters (only title and artist)
    let mut v_fuzzy = fetch_musixmatch_fuzzy(client, title, artist, &current_token, &current_cookie).await?;

    // Check for 401 error in fuzzy response
    status_code = v_fuzzy
        .get("message")
        .and_then(|m| m.get("header"))
        .and_then(|h| h.get("status_code"))
        .and_then(|s| s.as_i64());

    if status_code == Some(401) {
        if let Some((t, cookie)) = get_musixmatch_token(client).await {
            current_token = t.clone();
            current_cookie = cookie.clone();

            // Update memory cache
            if let Ok(mut guard) = CACHED_TOKEN.lock() {
                *guard = Some(t);
            }
            if let Ok(mut guard) = CACHED_COOKIE.lock() {
                *guard = Some(cookie);
            }
            v_fuzzy = fetch_musixmatch_fuzzy(client, title, artist, &current_token, &current_cookie).await?;
        }
    }

    if let Some(lyrics) = parse_musixmatch_value(&v_fuzzy) {
        return Some(lyrics);
    }

    // Second Fallback: retry with cleaned title (strip parentheses/brackets)
    let cleaned_title = clean_song_title(title);
    if cleaned_title != title {
        let mut v_cleaned = fetch_musixmatch_fuzzy(client, &cleaned_title, artist, &current_token, &current_cookie).await?;

        // Check for 401 error in cleaned response
        status_code = v_cleaned
            .get("message")
            .and_then(|m| m.get("header"))
            .and_then(|h| h.get("status_code"))
            .and_then(|s| s.as_i64());

        if status_code == Some(401) {
            if let Some((t, cookie)) = get_musixmatch_token(client).await {
                current_token = t.clone();
                current_cookie = cookie.clone();
                
                // Update memory cache
                if let Ok(mut guard) = CACHED_TOKEN.lock() {
                    *guard = Some(t);
                }
                if let Ok(mut guard) = CACHED_COOKIE.lock() {
                    *guard = Some(cookie);
                }
                v_cleaned = fetch_musixmatch_fuzzy(client, &cleaned_title, artist, &current_token, &current_cookie).await?;
            }
        }

        if let Some(lyrics) = parse_musixmatch_value(&v_cleaned) {
            return Some(lyrics);
        }
    }

    None
}
