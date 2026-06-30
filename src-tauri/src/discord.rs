// Discord Rich Presence — shows the current track as your Discord status.
//
// Talks to the local Discord client over IPC (no network/token). Cross-platform.
// The Application ID controls the app name shown ("Listening to <app name>") and
// which uploaded art assets are available; the user supplies their own from the
// Discord Developer Portal (https://discord.com/developers/applications). With no
// ID set, Rich Presence simply stays disconnected.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

pub struct DiscordState(pub Mutex<DiscordInner>);

pub struct DiscordInner {
    client: Option<DiscordIpcClient>,
    enabled: bool,
    client_id: String,
}

impl DiscordState {
    pub fn new() -> Self {
        DiscordState(Mutex::new(DiscordInner {
            client: None,
            enabled: false,
            client_id: String::new(),
        }))
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// Connect (or reconnect) using the stored client id. No-op when disabled or the
// id is blank. Returns an error string the UI can surface.
fn ensure_connected(inner: &mut DiscordInner) -> Result<(), String> {
    if !inner.enabled || inner.client_id.trim().is_empty() {
        return Ok(());
    }
    if inner.client.is_some() {
        return Ok(());
    }
    let mut client = DiscordIpcClient::new(inner.client_id.trim());
    client.connect().map_err(|e| e.to_string())?;
    inner.client = Some(client);
    Ok(())
}

fn disconnect(inner: &mut DiscordInner) {
    if let Some(mut client) = inner.client.take() {
        let _ = client.close();
    }
}

// Enable/disable Rich Presence and set the application id. Connects immediately
// when enabling so errors (e.g. Discord not running) surface to the caller.
#[tauri::command]
pub fn discord_set_enabled(
    state: tauri::State<DiscordState>,
    enabled: bool,
    client_id: String,
) -> Result<(), String> {
    let mut inner = state.0.lock().map_err(|_| "discord state poisoned")?;
    let id_changed = inner.client_id != client_id;
    inner.enabled = enabled;
    inner.client_id = client_id;

    if !enabled {
        disconnect(&mut inner);
        return Ok(());
    }
    // Reconnect if the id changed while enabled.
    if id_changed {
        disconnect(&mut inner);
    }
    ensure_connected(&mut inner)
}

// Push the now-playing track. Silent no-op when disabled/unconnected so the
// frontend can call it freely on every track / play-pause change.
#[tauri::command]
pub fn discord_update(
    state: tauri::State<DiscordState>,
    title: String,
    artist: String,
    album: String,
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

    let details = if title.trim().is_empty() { "Unknown title".to_string() } else { title };
    let state_line = if artist.trim().is_empty() { "Unknown artist".to_string() } else { artist };

    let mut assets = activity::Assets::new().large_image("logo");
    if !album.trim().is_empty() {
        assets = assets.large_text(album.as_str());
    }

    // Progress bar timestamps (Discord expects Unix ms). Only while playing.
    let start = now_ms() - (position.max(0.0) * 1000.0) as i64;
    let end = start + (duration.max(0.0) * 1000.0) as i64;

    let mut payload = activity::Activity::new()
        .activity_type(activity::ActivityType::Listening)
        .details(details.as_str())
        .state(state_line.as_str())
        .assets(assets);

    if is_playing && duration > 0.0 {
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
