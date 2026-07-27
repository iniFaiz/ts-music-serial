//! Native consent grants for high-impact filesystem and database operations.
//!
//! A webview confirmation is useful UX but is not a security boundary. These
//! short-lived, single-use grants are issued only after an operating-system
//! dialog and are bound to a fixed action plus canonical target paths.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::{db, limits, resolve_allowed_audio};

const CONSENT_TTL: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsentAction {
    DeleteAudio,
    DeletePlaylist,
    RemoveLibraryTracks,
    RemoveLibraryRoot,
    WriteTrackTags,
    ImportOnlineMetadata,
    ResetLibrary,
    ImportBackup,
}

impl ConsentAction {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "delete_audio" => Ok(Self::DeleteAudio),
            "delete_playlist" => Ok(Self::DeletePlaylist),
            "remove_library_tracks" => Ok(Self::RemoveLibraryTracks),
            "remove_library_root" => Ok(Self::RemoveLibraryRoot),
            "write_track_tags" => Ok(Self::WriteTrackTags),
            "import_online_metadata" => Ok(Self::ImportOnlineMetadata),
            "reset_library" => Ok(Self::ResetLibrary),
            "import_backup" => Ok(Self::ImportBackup),
            _ => Err("Unsupported destructive action".to_string()),
        }
    }
}

struct ConsentGrant {
    action: ConsentAction,
    remaining_targets: HashSet<String>,
    targetless: bool,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct DestructiveConsentState {
    grants: Mutex<HashMap<String, ConsentGrant>>,
}

impl DestructiveConsentState {
    fn issue(
        &self,
        action: ConsentAction,
        targets: Vec<String>,
        targetless: bool,
    ) -> Result<String, String> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)
            .map_err(|error| format!("Failed to create consent token: {error}"))?;
        let token = hex::encode(random);
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, grant| grant.expires_at > now);
        grants.insert(
            token.clone(),
            ConsentGrant {
                action,
                remaining_targets: targets.into_iter().collect(),
                targetless,
                expires_at: now + CONSENT_TTL,
            },
        );
        Ok(token)
    }

    pub(crate) fn consume(
        &self,
        token: &str,
        action: ConsentAction,
        target: Option<&str>,
    ) -> Result<(), String> {
        let now = Instant::now();
        let mut grants = self.grants.lock();
        grants.retain(|_, grant| grant.expires_at > now);
        let grant = grants
            .get_mut(token)
            .ok_or_else(|| "Native consent is missing or expired".to_string())?;
        if grant.action != action {
            return Err("Native consent does not match this operation".to_string());
        }

        match target {
            Some(target) if grant.remaining_targets.remove(target) => {}
            Some(_) => return Err("Native consent does not cover this target".to_string()),
            None if grant.targetless => grant.targetless = false,
            None => return Err("Native consent does not cover this operation".to_string()),
        }

        if grant.remaining_targets.is_empty() && !grant.targetless {
            grants.remove(token);
        }
        Ok(())
    }
}

fn canonical_backup(app: &AppHandle, value: &str) -> Result<String, String> {
    limits::validate_text(value, "Backup path", limits::MAX_PATH_BYTES)?;
    let path = Path::new(value);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension != "db" && extension != "tsmback" {
        return Err("Backup path must end in .db or .tsmback".to_string());
    }
    if !crate::is_allowed_path(app, path) {
        return Err("Backup path was not authorized by the file picker".to_string());
    }
    crate::library_scan::canonicalize_existing_path(path)
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| format!("Backup file is unavailable: {error}"))
}

fn registered_root(app: &AppHandle, value: &str) -> Result<String, String> {
    limits::validate_text(value, "Library root", limits::MAX_PATH_BYTES)?;
    let stored = db::roots(app.state::<db::Db>().inner())?;
    let requested = crate::library_scan::canonicalize_directory(Path::new(value)).ok();
    stored
        .into_iter()
        .find(|candidate| {
            candidate == value
                || requested.as_ref().is_some_and(|requested| {
                    crate::library_scan::canonicalize_directory(Path::new(candidate))
                        .is_ok_and(|saved| saved == *requested)
                })
        })
        .ok_or_else(|| "Library root is not registered".to_string())
}

fn registered_playlist(app: &AppHandle, value: &str) -> Result<(String, String), String> {
    limits::validate_text(value, "Playlist ID", 128)?;
    let database = app.state::<db::Db>();
    let connection = database.0.lock();
    let name = connection
        .query_row("SELECT name FROM playlists WHERE id = ?1", [value], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Playlist does not exist".to_string())?;
    Ok((value.to_string(), name))
}

fn normalize_audio_targets(app: &AppHandle, targets: &[String]) -> Result<Vec<String>, String> {
    limits::validate_paths(targets, limits::MAX_QUEUE_ENTRIES)?;
    targets
        .iter()
        .map(|path| {
            resolve_allowed_audio(app, Path::new(path))
                .map(|path| path.to_string_lossy().to_string())
        })
        .collect()
}

fn target_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .chars()
        .take(160)
        .collect()
}

#[tauri::command]
pub(crate) async fn request_destructive_consent(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, DestructiveConsentState>,
    action: String,
    targets: Vec<String>,
) -> Result<Option<String>, String> {
    if window.label() != "main" {
        return Err("Destructive consent is available only to the main window".to_string());
    }
    let action = ConsentAction::parse(&action)?;
    let (normalized, targetless, title, message, confirm_label) = match action {
        ConsentAction::DeleteAudio => {
            if targets.is_empty() {
                return Err("At least one audio file is required".to_string());
            }
            let normalized = normalize_audio_targets(&app, &targets)?;
            let message = if normalized.len() == 1 {
                format!(
                    "Move \"{}\" to the operating-system Recycle Bin/Trash?",
                    target_name(&normalized[0])
                )
            } else {
                format!(
                    "Move {} selected audio files to the operating-system Recycle Bin/Trash?",
                    normalized.len()
                )
            };
            (
                normalized,
                false,
                "Delete audio file",
                message,
                "Move to Trash",
            )
        }
        ConsentAction::DeletePlaylist => {
            if targets.len() != 1 {
                return Err("Exactly one playlist ID is required".to_string());
            }
            let (id, name) = registered_playlist(&app, &targets[0])?;
            (
                vec![id],
                false,
                "Delete playlist",
                format!(
                    "Delete the playlist \"{}\" and remove all of its items?",
                    name.chars().take(200).collect::<String>()
                ),
                "Delete Playlist",
            )
        }
        ConsentAction::RemoveLibraryTracks => {
            if targets.is_empty() {
                return Err("At least one audio file is required".to_string());
            }
            let normalized = normalize_audio_targets(&app, &targets)?;
            let message = if normalized.len() == 1 {
                format!(
                    "Remove \"{}\" and its listening history from the TS Music library? The audio file will remain on disk.",
                    target_name(&normalized[0])
                )
            } else {
                format!(
                    "Remove {} selected tracks and their listening history from the TS Music library? The audio files will remain on disk.",
                    normalized.len()
                )
            };
            (
                normalized,
                false,
                "Remove tracks from library",
                message,
                "Remove from Library",
            )
        }
        ConsentAction::RemoveLibraryRoot => {
            if targets.len() != 1 {
                return Err("Exactly one library root is required".to_string());
            }
            let root = registered_root(&app, &targets[0])?;
            let message = format!(
                "Remove this folder and all of its indexed tracks from TS Music?\n\n{root}"
            );
            (vec![root], false, "Remove music folder", message, "Remove")
        }
        ConsentAction::WriteTrackTags => {
            if targets.len() != 1 {
                return Err("Exactly one audio file is required".to_string());
            }
            let normalized = normalize_audio_targets(&app, &targets)?;
            let message = format!(
                "Write metadata changes directly into \"{}\"?",
                target_name(&normalized[0])
            );
            (
                normalized,
                false,
                "Modify audio file",
                message,
                "Write Tags",
            )
        }
        ConsentAction::ImportOnlineMetadata => {
            if !targets.is_empty() {
                return Err("Online metadata consent does not accept targets".to_string());
            }
            (
                Vec::new(),
                true,
                "Modify missing metadata",
                "Allow this scan to write missing tags and artwork into your indexed audio files?"
                    .to_string(),
                "Allow This Scan",
            )
        }
        ConsentAction::ResetLibrary => {
            if !targets.is_empty() {
                return Err("Reset consent does not accept targets".to_string());
            }
            (
                Vec::new(),
                true,
                "Reset TS Music library",
                "Clear all indexed tracks, roots, favorites, playlists, and listening history?"
                    .to_string(),
                "Reset Library",
            )
        }
        ConsentAction::ImportBackup => {
            if targets.len() != 1 {
                return Err("Exactly one backup file is required".to_string());
            }
            let backup = canonical_backup(&app, &targets[0])?;
            let message = format!(
                "Replace the current TS Music database with this backup?\n\n{}",
                target_name(&backup)
            );
            (
                vec![backup],
                false,
                "Restore database backup",
                message,
                "Restore Backup",
            )
        }
    };

    let dialog = app
        .dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            confirm_label.to_string(),
            "Cancel".to_string(),
        ))
        .parent(&window);
    let confirmed = tauri::async_runtime::spawn_blocking(move || dialog.blocking_show())
        .await
        .map_err(|error| format!("Native confirmation failed: {error}"))?;

    if !confirmed {
        return Ok(None);
    }
    state.issue(action, normalized, targetless).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_is_bound_to_action_target_and_single_use() {
        let state = DestructiveConsentState::default();
        let token = state
            .issue(
                ConsentAction::DeleteAudio,
                vec!["C:/Music/one.flac".to_string()],
                false,
            )
            .expect("issue grant");

        assert!(state
            .consume(
                &token,
                ConsentAction::DeleteAudio,
                Some("C:/Music/two.flac")
            )
            .is_err());
        assert!(state
            .consume(
                &token,
                ConsentAction::DeleteAudio,
                Some("C:/Music/one.flac")
            )
            .is_ok());
        assert!(state
            .consume(
                &token,
                ConsentAction::DeleteAudio,
                Some("C:/Music/one.flac")
            )
            .is_err());
    }

    #[test]
    fn targetless_grant_is_single_use() {
        let state = DestructiveConsentState::default();
        let token = state
            .issue(ConsentAction::ResetLibrary, Vec::new(), true)
            .expect("issue grant");
        assert!(state
            .consume(&token, ConsentAction::ResetLibrary, None)
            .is_ok());
        assert!(state
            .consume(&token, ConsentAction::ResetLibrary, None)
            .is_err());
    }
}
