//! Bounded-memory native library indexing.
//!
//! Discovery, metadata parsing, fingerprinting, and SQLite writes stay inside
//! one serialized Rust job. The webview receives only an `IndexSummary`, never
//! a full `Vec<MusicTrack>`.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use walkdir::WalkDir;

use crate::library_db as db;
use crate::library_scan::{
    canonical_roots, canonicalize_directory, canonicalize_existing_path, consume_native_drop,
    file_identity, grant_session_audio, path_is_within_roots, register_library_roots,
};
use crate::{is_audio_file, parse_metadata, MusicTrack};

// Metadata objects (and their tag-reader allocations) are released after every
// transaction rather than accumulating for the entire library.
const INDEX_BATCH_SIZE: usize = 96;
const _: () = assert!(INDEX_BATCH_SIZE > 0 && INDEX_BATCH_SIZE <= 256);

pub(crate) struct LibraryIndexState {
    pub(crate) job: Mutex<()>,
    // Number of currently running indexing jobs (full scans and watcher
    // refreshes). Background maintenance polls this so it can stand down
    // without holding — and stalling — the job mutex itself.
    active_jobs: AtomicUsize,
}

impl LibraryIndexState {
    pub(crate) fn new() -> Self {
        Self {
            job: Mutex::new(()),
            active_jobs: AtomicUsize::new(0),
        }
    }
}

// RAII decrement: a panic inside an indexing job must not leak the count and
// permanently idle the background backfill.
struct ActiveJobGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveJobGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// True while any foreground indexing job is running. Background jobs
/// (fingerprint backfill) poll this between batches instead of taking the
/// global job mutex, which would make watcher updates and manual reindexes
/// queue behind them.
pub(crate) fn index_jobs_active(app: &AppHandle) -> bool {
    app.try_state::<LibraryIndexState>()
        .map(|state| state.active_jobs.load(Ordering::SeqCst) > 0)
        .unwrap_or(false)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexSummary {
    pub(crate) scanned: usize,
    pub(crate) added: usize,
    pub(crate) removed: usize,
    pub(crate) total: i64,
    pub(crate) duration_ms: u128,
    // Only input roots (normally one or a handful), not every indexed track.
    pub(crate) roots: Vec<String>,
}

fn flush_batch(
    app: &AppHandle,
    batch: &mut Vec<PathBuf>,
    use_parallelism: bool,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }

    let paths = mem::take(batch);
    let signatures = paths
        .into_iter()
        .filter_map(|path| {
            let metadata = path.metadata().ok()?;
            let mtime_ns = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
                .unwrap_or(0);
            let file_id = file_identity(&path, &metadata);
            Some((
                path,
                metadata.len().min(i64::MAX as u64) as i64,
                mtime_ns,
                file_id,
            ))
        })
        .collect::<Vec<_>>();
    summary.scanned += signatures.len();
    let paths = db::paths_requiring_metadata(&app.state::<db::Db>(), &signatures)?;
    let tracks: Vec<MusicTrack> = if use_parallelism {
        paths
            .into_par_iter()
            .filter_map(|path| parse_metadata(&path))
            .collect()
    } else {
        paths
            .into_iter()
            .filter_map(|path| parse_metadata(&path))
            .collect()
    };

    summary.added += db::upsert_tracks(&app.state::<db::Db>(), tracks)?;
    Ok(())
}

fn queue_audio_path(
    app: &AppHandle,
    path: PathBuf,
    use_parallelism: bool,
    batch: &mut Vec<PathBuf>,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if !is_audio_file(&path) {
        return Ok(());
    }
    batch.push(path);
    if batch.len() >= INDEX_BATCH_SIZE {
        flush_batch(app, batch, use_parallelism, summary)?;
    }
    Ok(())
}

fn index_directory(
    app: &AppHandle,
    root: &Path,
    use_parallelism: bool,
    batch: &mut Vec<PathBuf>,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if use_parallelism {
        // jwalk parallelises directory IO, while bounded batches parallelise tag
        // parsing. Crucially, the iterator is consumed as a stream.
        for entry in jwalk::WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                if let Some(path) = safe_walked_file(root, &entry.path()) {
                    queue_audio_path(app, path, use_parallelism, batch, summary)?;
                }
            }
        }
    } else {
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                if let Some(path) = safe_walked_file(root, entry.path()) {
                    queue_audio_path(app, path, use_parallelism, batch, summary)?;
                }
            }
        }
    }
    Ok(())
}

/// Resolve every file found by the walker and verify the resolved target still
/// lies below the canonical root. Walkers do not follow symlinks by default;
/// this second check also covers Windows junction/reparse-point surprises and
/// a link swap between directory enumeration and metadata parsing.
fn safe_walked_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    if !is_audio_file(candidate) {
        return None;
    }
    let canonical = canonicalize_existing_path(candidate).ok()?;
    if canonical.is_file()
        && is_audio_file(&canonical)
        && path_is_within_roots(&canonical, &[root.to_owned()])
    {
        Some(canonical)
    } else {
        None
    }
}

fn push_root(roots: &mut Vec<String>, root: &Path) {
    let value = root.to_string_lossy().to_string();
    if !roots.iter().any(|existing| existing == &value) {
        roots.push(value);
    }
}

fn compact_changed_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort_by_key(|path| path.components().count());
    let mut compacted: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        let covered_by_parent = compacted
            .iter()
            .any(|parent| parent != &path && !is_audio_file(parent) && path.starts_with(parent));
        if !covered_by_parent && !compacted.contains(&path) {
            compacted.push(path);
        }
    }
    compacted
}

fn index_paths_locked(
    app: &AppHandle,
    paths: Vec<PathBuf>,
    use_parallelism: bool,
    prune_missing: bool,
) -> Result<IndexSummary, String> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    let mut batch = Vec::with_capacity(INDEX_BATCH_SIZE);

    for path in paths {
        if path.is_dir() {
            let before = summary.scanned;
            index_directory(app, &path, use_parallelism, &mut batch, &mut summary)?;
            flush_batch(app, &mut batch, use_parallelism, &mut summary)?;
            if summary.scanned > before {
                push_root(&mut summary.roots, &path);
            }
        } else if path.is_file() && is_audio_file(&path) {
            queue_audio_path(app, path, use_parallelism, &mut batch, &mut summary)?;
        }
    }
    flush_batch(app, &mut batch, use_parallelism, &mut summary)?;

    if prune_missing {
        summary.removed = db::db_prune_missing(app.state::<db::Db>())?.len();
    }
    summary.total = db::db_count(app.state::<db::Db>())?;
    summary.duration_ms = started.elapsed().as_millis();
    Ok(summary)
}

pub(crate) fn index_authorized_paths(
    app: &AppHandle,
    paths: Vec<PathBuf>,
    use_parallelism: bool,
    prune_missing: bool,
) -> Result<IndexSummary, String> {
    let state = app.state::<LibraryIndexState>();
    let _job = state.job.lock();
    let _active = ActiveJobGuard(&state.active_jobs);
    index_paths_locked(app, paths, use_parallelism, prune_missing)
}

// Full/manual scans never accept webview-provided paths. With no grant they
// read the canonical roots from SQLite. A native drag/drop token is consumed
// exactly once and resolves to the paths captured by Tauri's native event.
#[tauri::command]
pub(crate) async fn index_library(
    app: AppHandle,
    use_parallelism: bool,
    prune_missing: bool,
    dnd_grant: Option<String>,
) -> Result<IndexSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let is_drop = dnd_grant.is_some();
        let mut roots_added = Vec::new();
        let paths = if let Some(token) = dnd_grant {
            let dropped = consume_native_drop(&app, &token)?;
            let mut directories = Vec::new();
            let mut files = Vec::new();
            for path in dropped {
                if path.is_dir() {
                    directories.push(canonicalize_directory(&path)?);
                } else if path.is_file() && is_audio_file(&path) {
                    files.push(grant_session_audio(&app, &path)?);
                }
            }
            if !directories.is_empty() {
                roots_added = register_library_roots(&app, &directories)?;
            }
            directories.extend(files);
            directories
        } else {
            canonical_roots(&app)
        };

        let mut summary =
            index_authorized_paths(&app, paths, use_parallelism, prune_missing && !is_drop)?;
        if is_drop {
            summary.roots = roots_added;
        }
        Ok(summary)
    })
    .await
    .map_err(|error| format!("Library index job failed: {error}"))?
}

// The watcher uses exactly the event paths. Existing directories are limited to
// that subtree; existing files are reparsed individually. A removal/rename only
// runs the DB's existence pruning and never traverses a library root.
pub(crate) fn index_watcher_paths(
    app: &AppHandle,
    paths: Vec<PathBuf>,
) -> Result<IndexSummary, String> {
    let state = app.state::<LibraryIndexState>();
    let _job = state.job.lock();
    let _active = ActiveJobGuard(&state.active_jobs);
    let roots = canonical_roots(app);
    let paths = compact_changed_paths(
        paths
            .into_iter()
            .filter_map(|path| {
                if path.exists() {
                    canonicalize_existing_path(&path)
                        .ok()
                        .filter(|resolved| path_is_within_roots(resolved, &roots))
                } else if path.is_absolute() && path_is_within_roots(&path, &roots) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect(),
    );
    let changed = paths.clone();
    let mut summary = index_paths_locked(app, paths, true, false)?;
    summary.removed = db::prune_changed_paths(&app.state::<db::Db>(), &changed)?.len();
    summary.total = db::db_count(app.state::<db::Db>())?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::compact_changed_paths;

    #[test]
    fn watcher_paths_collapse_children_under_a_changed_directory() {
        let root = PathBuf::from("library").join("new-album");
        let song = root.join("song.flac");
        let cover = root.join("cover.jpg");
        assert_eq!(
            compact_changed_paths(vec![song, cover, root.clone()]),
            vec![root]
        );
    }

    #[test]
    fn watcher_paths_keep_independent_audio_files() {
        let first = PathBuf::from("library").join("one.flac");
        let second = PathBuf::from("library").join("two.flac");
        let compacted = compact_changed_paths(vec![first.clone(), second.clone()]);
        assert!(compacted.contains(&first));
        assert!(compacted.contains(&second));
    }
}
