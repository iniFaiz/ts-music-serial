//! Bounded-memory native library indexing.
//!
//! Discovery, metadata parsing, fingerprinting, and SQLite writes stay inside
//! one serialized Rust job. The webview receives only an `IndexSummary`, never
//! a full `Vec<MusicTrack>`.

use std::mem;
use std::path::{Path, PathBuf};
use std::time::Instant;

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use walkdir::WalkDir;

use crate::library_db as db;
use crate::{allow_root, is_audio_file, parse_metadata, MusicTrack};

// Metadata objects (and their tag-reader allocations) are released after every
// transaction rather than accumulating for the entire library.
const INDEX_BATCH_SIZE: usize = 96;

pub(crate) struct LibraryIndexState {
    pub(crate) job: Mutex<()>,
}

impl LibraryIndexState {
    pub(crate) fn new() -> Self {
        Self {
            job: Mutex::new(()),
        }
    }
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
    refresh_fingerprints: bool,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }

    let paths = mem::take(batch);
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

    summary.scanned += tracks.len();
    summary.added += if refresh_fingerprints {
        db::upsert_changed_tracks(&app.state::<db::Db>(), tracks)?
    } else {
        db::upsert_tracks(&app.state::<db::Db>(), tracks)?
    };
    Ok(())
}

fn queue_audio_path(
    app: &AppHandle,
    path: PathBuf,
    use_parallelism: bool,
    refresh_fingerprints: bool,
    batch: &mut Vec<PathBuf>,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if !is_audio_file(&path) {
        return Ok(());
    }
    batch.push(path);
    if batch.len() >= INDEX_BATCH_SIZE {
        flush_batch(app, batch, use_parallelism, refresh_fingerprints, summary)?;
    }
    Ok(())
}

fn index_directory(
    app: &AppHandle,
    root: &Path,
    use_parallelism: bool,
    refresh_fingerprints: bool,
    batch: &mut Vec<PathBuf>,
    summary: &mut IndexSummary,
) -> Result<(), String> {
    if use_parallelism {
        // jwalk parallelises directory IO, while bounded batches parallelise tag
        // parsing. Crucially, the iterator is consumed as a stream.
        for entry in jwalk::WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                queue_audio_path(
                    app,
                    entry.path(),
                    use_parallelism,
                    refresh_fingerprints,
                    batch,
                    summary,
                )?;
            }
        }
    } else {
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                queue_audio_path(
                    app,
                    entry.path().to_owned(),
                    use_parallelism,
                    refresh_fingerprints,
                    batch,
                    summary,
                )?;
            }
        }
    }
    Ok(())
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
    refresh_fingerprints: bool,
) -> Result<IndexSummary, String> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    let mut batch = Vec::with_capacity(INDEX_BATCH_SIZE);

    for path in paths {
        if path.is_dir() {
            allow_root(app, &path.to_string_lossy());
            let before = summary.scanned;
            index_directory(
                app,
                &path,
                use_parallelism,
                refresh_fingerprints,
                &mut batch,
                &mut summary,
            )?;
            flush_batch(
                app,
                &mut batch,
                use_parallelism,
                refresh_fingerprints,
                &mut summary,
            )?;
            // A dropped empty directory is not registered automatically; a
            // folder explicitly selected by the user is added by the frontend.
            if summary.scanned > before {
                push_root(&mut summary.roots, &path);
            }
        } else if path.is_file() && is_audio_file(&path) {
            if let Some(parent) = path.parent() {
                allow_root(app, &parent.to_string_lossy());
                push_root(&mut summary.roots, parent);
            }
            queue_audio_path(
                app,
                path,
                use_parallelism,
                refresh_fingerprints,
                &mut batch,
                &mut summary,
            )?;
        }
    }
    flush_batch(
        app,
        &mut batch,
        use_parallelism,
        refresh_fingerprints,
        &mut summary,
    )?;

    if prune_missing {
        summary.removed = db::db_prune_missing(app.state::<db::Db>())?.len();
    }
    summary.total = db::db_count(app.state::<db::Db>())?;
    summary.duration_ms = started.elapsed().as_millis();
    Ok(summary)
}

// Full/manual scans enter here. `spawn_blocking` keeps traversal and tag reads
// off Tauri's async runtime; `job` guarantees full scans and watcher batches do
// not race each other or multiply memory usage.
#[tauri::command]
pub(crate) async fn index_library(
    app: AppHandle,
    paths: Vec<String>,
    use_parallelism: bool,
    prune_missing: bool,
) -> Result<IndexSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LibraryIndexState>();
        let _job = state.job.lock();
        let paths = paths.into_iter().map(PathBuf::from).collect();
        index_paths_locked(&app, paths, use_parallelism, prune_missing, false)
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
    let paths = compact_changed_paths(paths);
    let changed = paths.clone();
    let mut summary = index_paths_locked(app, paths, true, false, true)?;
    summary.removed = db::prune_changed_paths(&app.state::<db::Db>(), &changed)?.len();
    summary.total = db::db_count(app.state::<db::Db>())?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{compact_changed_paths, INDEX_BATCH_SIZE};

    #[test]
    fn indexing_uses_a_bounded_nonzero_batch() {
        assert!(INDEX_BATCH_SIZE > 0);
        assert!(INDEX_BATCH_SIZE <= 256);
    }

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
