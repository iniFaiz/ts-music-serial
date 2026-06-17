// ---------------------------------------------------------------------------
// Lyrics providers + LRC parsing
//
// All network access lives here (in Rust) rather than in the webview, so the
// app's strict CSP stays intact. The orchestration / caching / local-tag lookup
// lives in lib.rs (which owns the asset-protocol scope check); this module
// provides the pure LRC parser and the remote provider fetchers.
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LyricLine {
    // Milliseconds from the start of the track. None for unsynced (plain) lyrics.
    pub time_ms: Option<u64>,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lyrics {
    pub synced: bool,
    pub source: String,
    pub lines: Vec<LyricLine>,
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
        });
    }
    let plain = make_plain(t);
    if plain.iter().any(|l| !l.text.trim().is_empty()) {
        Some(Lyrics {
            synced: false,
            source: source.to_string(),
            lines: plain,
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

pub async fn from_netease(client: &reqwest::Client, title: &str, artist: &str) -> Option<Lyrics> {
    let q = format!("{title} {artist}");
    let search = client
        .post("https://music.163.com/api/search/get/web")
        .form(&[
            ("s", q.as_str()),
            ("type", "1"),
            ("limit", "5"),
            ("offset", "0"),
        ])
        .header("Referer", "https://music.163.com")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = search.json().await.ok()?;
    let songs = v.get("result")?.get("songs")?.as_array()?;
    let id = songs.first()?.get("id")?.as_i64()?;
    let ids = id.to_string();

    let lyric = client
        .get("https://music.163.com/api/song/lyric")
        .query(&[
            ("os", "pc"),
            ("id", ids.as_str()),
            ("lv", "-1"),
            ("kv", "-1"),
            ("tv", "-1"),
        ])
        .header("Referer", "https://music.163.com")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .ok()?;
    let lv: serde_json::Value = lyric.json().await.ok()?;
    let lrc = lv.get("lrc")?.get("lyric")?.as_str()?;
    lyrics_from_text(lrc, "NetEase")
}

// ---- Provider: Musixmatch (requires a community user token) ----------------

pub async fn from_musixmatch(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    album: &str,
    duration: u64,
    token: &str,
) -> Option<Lyrics> {
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
        .header("Cookie", "AWSELB=0; AWSELBCORS=0; x-mxm-token-guid=")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Musixmatch/0.19.4 Chrome/58.0.3029.110 Electron/1.7.6 Safari/537.36")
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    
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
