use serde::Serialize;
use std::io::Write;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

use crate::limits;

const UPDATE_ENDPOINT: &str =
    "https://github.com/iniFaiz/ts-music-serial/releases/latest/download/latest.json";
const UPDATE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterStatus {
    configured: bool,
    current_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current_version: String,
    version: String,
    body: Option<String>,
    published_at: Option<String>,
    rollout_percentage: u8,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
    finished: bool,
}

fn public_key() -> Option<&'static str> {
    option_env!("TS_MUSIC_UPDATER_PUBLIC_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn configured_updater(app: &AppHandle) -> Result<Updater, String> {
    let key = public_key().ok_or_else(|| {
        "Updater is disabled in this build because no signing public key was embedded".to_string()
    })?;
    let endpoint = UPDATE_ENDPOINT
        .parse()
        .map_err(|error| format!("Invalid updater endpoint: {error}"))?;
    app.updater_builder()
        .pubkey(key)
        .timeout(UPDATE_TIMEOUT)
        .endpoints(vec![endpoint])
        .map_err(|error| format!("Invalid updater configuration: {error}"))?
        .build()
        .map_err(|error| format!("Failed to initialize updater: {error}"))
}

fn update_info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        body: update
            .body
            .as_deref()
            .map(|body| body.chars().take(16_000).collect()),
        published_at: update.date.map(|date| date.to_string()),
        rollout_percentage: rollout_percentage(update),
    }
}

fn rollout_percentage(update: &Update) -> u8 {
    update
        .raw_json
        .get("rolloutPercentage")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(100)
        .min(100) as u8
}

fn rollout_cohort(app: &AppHandle) -> Result<u8, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Cannot resolve updater state directory: {error}"))?;
    let cohort_path = app_data.join("updater-cohort");
    if let Ok(value) = std::fs::read_to_string(&cohort_path) {
        if let Ok(cohort) = value.trim().parse::<u8>() {
            if cohort < 100 {
                return Ok(cohort);
            }
        }
    }

    std::fs::create_dir_all(&app_data)
        .map_err(|error| format!("Cannot create updater state directory: {error}"))?;
    let cohort = loop {
        let mut random = [0_u8; 1];
        getrandom::fill(&mut random)
            .map_err(|error| format!("Cannot generate updater rollout cohort: {error}"))?;
        if random[0] < 200 {
            break random[0] % 100;
        }
    };

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cohort_path)
    {
        Ok(mut file) => {
            file.write_all(cohort.to_string().as_bytes())
                .map_err(|error| format!("Cannot persist updater rollout cohort: {error}"))?;
            Ok(cohort)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let value = std::fs::read_to_string(&cohort_path)
                .map_err(|error| format!("Cannot read updater rollout cohort: {error}"))?;
            value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|value| *value < 100)
                .ok_or_else(|| "Stored updater rollout cohort is invalid".to_string())
        }
        Err(error) => Err(format!(
            "Cannot create updater rollout cohort file: {error}"
        )),
    }
}

fn rollout_allows(app: &AppHandle, update: &Update) -> Result<bool, String> {
    Ok(rollout_cohort(app)? < rollout_percentage(update))
}

#[tauri::command]
pub fn updater_status(app: AppHandle) -> UpdaterStatus {
    UpdaterStatus {
        configured: public_key().is_some(),
        current_version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub async fn updater_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = configured_updater(&app)?
        .check()
        .await
        .map_err(|error| format!("Update check failed: {error}"))?;
    match update {
        Some(update) if rollout_allows(&app, &update)? => Ok(Some(update_info(&update))),
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn updater_install(app: AppHandle, expected_version: String) -> Result<(), String> {
    limits::validate_text(&expected_version, "Update version", 64)?;
    if !expected_version
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err("Update version contains unsupported characters".to_string());
    }

    // Fetch the feed again immediately before installation. This prevents stale
    // frontend state from selecting a different release than the one displayed.
    let update = configured_updater(&app)?
        .check()
        .await
        .map_err(|error| format!("Update check failed: {error}"))?
        .ok_or_else(|| "The selected update is no longer available".to_string())?;
    if !rollout_allows(&app, &update)? {
        return Err("This release is not available to this rollout cohort yet".to_string());
    }
    if update.version != expected_version {
        return Err(format!(
            "The available update changed from {expected_version} to {}. Check again before installing.",
            update.version
        ));
    }

    let downloaded = Arc::new(AtomicU64::new(0));
    let chunk_total = Arc::clone(&downloaded);
    let progress_app = app.clone();
    let finish_app = app.clone();
    update
        .download_and_install(
            move |chunk_size, total| {
                let bytes =
                    chunk_total.fetch_add(chunk_size as u64, Ordering::Relaxed) + chunk_size as u64;
                let _ = progress_app.emit(
                    "updater-progress",
                    UpdateProgress {
                        downloaded: bytes,
                        total,
                        finished: false,
                    },
                );
            },
            move || {
                let _ = finish_app.emit(
                    "updater-progress",
                    UpdateProgress {
                        downloaded: downloaded.load(Ordering::Relaxed),
                        total: None,
                        finished: true,
                    },
                );
            },
        )
        .await
        .map_err(|error| format!("Signed update could not be installed: {error}"))?;

    app.restart()
}

#[cfg(test)]
mod tests {
    use super::public_key;

    #[test]
    fn empty_compile_time_key_never_enables_updater() {
        if option_env!("TS_MUSIC_UPDATER_PUBLIC_KEY").is_some_and(str::is_empty) {
            assert!(public_key().is_none());
        }
    }
}
