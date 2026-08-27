//! Process-wide structured logging.
//!
//! One `tracing` subscriber fans out to two layers:
//! * stderr — immediate visibility in dev consoles and `tauri dev`.
//! * a daily-rotating file under `<app-data>/logs/ts-music.log.YYYY-MM-DD`,
//!   written by a non-blocking worker so a slow disk can never stall playback
//!   or indexing on a log line. The worker's guard is managed as Tauri state
//!   for the process lifetime; dropping it would discard buffered lines.
//!
//! Level defaults: `info` globally with `debug` for this crate, overridable
//! per-session via the standard `RUST_LOG` variable.

use std::path::Path;
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// How many daily log files stay on disk before startup cleanup removes them.
const LOG_RETENTION_DAYS: u64 = 7;

/// Install the global subscriber. Returns the non-blocking writer guard that
/// must stay alive for the remainder of the process (managed as Tauri state).
pub(crate) fn init(app_data: Option<&Path>) -> WorkerGuard {
    let logs_dir = app_data.map(|dir| dir.join("logs"));
    if let Some(dir) = logs_dir.as_ref() {
        if let Err(error) = std::fs::create_dir_all(dir) {
            eprintln!(
                "Could not create log directory '{}': {error}",
                dir.display()
            );
        }
        prune_expired_logs(dir);
    }

    // When no data dir is available (should not happen) the file layer still
    // writes under the current working directory rather than being disabled,
    // so diagnostics never silently vanish.
    let file_target = logs_dir.unwrap_or_else(|| std::path::PathBuf::from("."));
    let file_worker = tracing_appender::rolling::daily(file_target, "ts-music.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_worker);

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,ts_music=debug"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(filter.clone());
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    guard
}

// Daily rotation never deletes by itself; drop files past the retention window
// once at startup. Best-effort: failures are non-fatal and left on disk.
fn prune_expired_logs(logs_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };
    let cutoff = SystemTime::now() - Duration::from_secs(LOG_RETENTION_DAYS * 24 * 3600);
    for entry in entries.flatten() {
        let path = entry.path();
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map(|modified| modified < cutoff)
            .unwrap_or(false);
        if expired && path.file_name().is_some_and(|name| {
            name.to_string_lossy().starts_with("ts-music.log")
        }) && path.extension().is_some() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_prune_removes_only_expired_ts_music_logs() {
        use std::fs::FileTimes;

        let dir = std::env::temp_dir().join(format!("ts-music-log-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp log dir");

        let fresh = dir.join("ts-music.log.2099-01-01");
        let stale = dir.join("ts-music.log.2000-01-01");
        let other = dir.join("unrelated.txt");
        std::fs::write(&fresh, b"keep").expect("write fresh");
        std::fs::write(&stale, b"drop").expect("write stale");
        // The pruner keys off modification time; backdate the "stale" file so
        // it is genuinely past the retention window despite being new on disk.
        let month_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
        let handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&stale)
            .expect("open stale for times");
        handle
            .set_times(FileTimes::new().set_modified(month_ago))
            .expect("backdate stale log");
        drop(handle);
        std::fs::write(&other, b"keep").expect("write other");

        prune_expired_logs(&dir);

        assert!(fresh.exists(), "recent log must survive retention pruning");
        assert!(!stale.exists(), "expired log must be pruned");
        assert!(other.exists(), "non-ts-music files are never touched");

        let _ = std::fs::remove_dir_all(dir);
    }
}
