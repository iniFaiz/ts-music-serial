// Discord Rich Presence — shows the current track as your Discord status.
//
// Talks to the local Discord client over IPC (no network/token). Cross-platform.
// The Application ID is hardcoded below so the user only has to toggle Rich
// Presence on/off — no Developer Portal setup required. Album art is resolved at
// runtime from the public iTunes Search API, so no art assets need uploading.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

// Our Discord application. Controls the app icon/name and base assets. Hardcoded
// on purpose: the user just flips the toggle in Settings.
const DISCORD_CLIENT_ID: &str = "1521515050160881775";

pub struct DiscordState(pub Mutex<DiscordInner>);

pub struct DiscordInner {
    client: Option<DiscordIpcClient>,
    enabled: bool,
}

impl DiscordState {
    pub fn new() -> Self {
        DiscordState(Mutex::new(DiscordInner {
            client: None,
            enabled: false,
        }))
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// Connect (or reconnect) using the hardcoded application id. No-op when disabled.
// Returns an error string the UI can surface.
fn ensure_connected(inner: &mut DiscordInner) -> Result<(), String> {
    if !inner.enabled {
        return Ok(());
    }
    if inner.client.is_some() {
        return Ok(());
    }
    let mut client = DiscordIpcClient::new(DISCORD_CLIENT_ID);
    client.connect().map_err(|e| e.to_string())?;
    inner.client = Some(client);
    Ok(())
}

fn disconnect(inner: &mut DiscordInner) {
    if let Some(mut client) = inner.client.take() {
        let _ = client.close();
    }
}

// Enable/disable Rich Presence. Connects immediately when enabling so errors
// (e.g. Discord not running) surface to the caller.
#[tauri::command]
pub fn discord_set_enabled(
    state: tauri::State<DiscordState>,
    enabled: bool,
) -> Result<(), String> {
    let mut inner = state.0.lock().map_err(|_| "discord state poisoned")?;
    inner.enabled = enabled;

    if !enabled {
        disconnect(&mut inner);
        return Ok(());
    }
    ensure_connected(&mut inner)
}

// Push the now-playing track. Silent no-op when disabled/unconnected so the
// frontend can call it freely on every track / play-pause change.
//
// When paused the presence is cleared entirely (the status disappears) rather
// than left frozen on the last track.
#[tauri::command]
pub fn discord_update(
    state: tauri::State<DiscordState>,
    title: String,
    artist: String,
    album: String,
    cover_url: String,
    is_playing: bool,
    position: f64,
    duration: f64,
) {
    let mut inner = match state.0.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if !inner.enabled {
        return;
    }
    if ensure_connected(&mut inner).is_err() || inner.client.is_none() {
        return;
    }

    // Paused → hide the presence (stay connected so resume re-shows instantly).
    if !is_playing {
        if let Some(client) = inner.client.as_mut() {
            let _ = client.clear_activity();
        }
        return;
    }

    let details = if title.trim().is_empty() { "Unknown title".to_string() } else { title };
    let state_line = if artist.trim().is_empty() { "Unknown artist".to_string() } else { artist };

    // Large image: the track's album art (a public https URL Discord proxies)
    // when we found one, otherwise fall back to the app's uploaded "logo" asset.
    let large_image = if cover_url.trim().is_empty() { "logo" } else { cover_url.trim() };
    let mut assets = activity::Assets::new().large_image(large_image);
    if !album.trim().is_empty() {
        assets = assets.large_text(album.as_str());
    }

    // Progress bar timestamps (Discord expects Unix ms).
    let start = now_ms() - (position.max(0.0) * 1000.0) as i64;
    let end = start + (duration.max(0.0) * 1000.0) as i64;

    let mut payload = activity::Activity::new()
        .activity_type(activity::ActivityType::Listening)
        // Show the artist in place of the app name ("Listening to <artist>").
        // `name` overrides the app name; `status_display_type(State)` makes the
        // member-list status line follow the `state` field (the artist) too.
        .name(state_line.as_str())
        .status_display_type(activity::StatusDisplayType::State)
        .details(details.as_str())
        .state(state_line.as_str())
        .assets(assets);

    if duration > 0.0 {
        payload = payload.timestamps(activity::Timestamps::new().start(start).end(end));
    }

    let client = inner.client.as_mut().unwrap();
    if client.set_activity(payload.clone()).is_err() {
        // Pipe may have dropped (Discord restarted) — try once to reconnect.
        if client.reconnect().is_ok() {
            let _ = client.set_activity(payload);
        }
    }
}

// Clear the presence (e.g. playback stopped) but stay connected.
#[tauri::command]
pub fn discord_clear(state: tauri::State<DiscordState>) {
    if let Ok(mut inner) = state.0.lock() {
        if let Some(client) = inner.client.as_mut() {
            let _ = client.clear_activity();
        }
    }
}

// Resolve album art for the current track via the public iTunes Search API
// (no key required). Returns a public https URL that Discord can proxy as the
// large image, or `None` when nothing matches (caller falls back to the logo).
#[tauri::command]
pub async fn discord_cover_art(title: String, artist: String, album: String) -> Option<String> {
    let artist = artist.trim();
    let album = album.trim();
    let title = title.trim();
    if artist.is_empty() && album.is_empty() && title.is_empty() {
        return None;
    }

    // Prefer an album lookup (stable art across a record); fall back to the track.
    let (entity, term) = if !album.is_empty() {
        ("album", format!("{} {}", artist, album))
    } else {
        ("musicTrack", format!("{} {}", artist, title))
    };

    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = HTTP
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("ts-music")
                .build()
                .unwrap_or_default()
        })
        .clone();

    let resp = client
        .get("https://itunes.apple.com/search")
        .query(&[
            ("term", term.as_str()),
            ("media", "music"),
            ("entity", entity),
            ("limit", "1"),
        ])
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let art = v.get("results")?.get(0)?.get("artworkUrl100")?.as_str()?;

    // iTunes serves a tiny 100x100 thumbnail; request a crisp 600x600 instead.
    Some(art.replace("100x100bb", "600x600bb"))
}
