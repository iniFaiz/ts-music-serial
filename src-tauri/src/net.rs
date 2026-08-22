//! Shared HTTP plumbing.
//!
//! Every network feature — lyric providers, cover-art lookups and the
//! MusicBrainz/AcoustID importer — funnels through one process-wide client so
//! pooled TCP/TLS connections are reused instead of rebuilt per feature. The
//! client carries the app's user agent (MusicBrainz requires an identifiable
//! one) plus a conservative default timeout; call sites override the timeout
//! per request when they need something different.

use std::sync::OnceLock;
use std::time::Duration;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// The process-wide client: one connection pool, rustls TLS, the app user agent
/// and a 15s default request timeout.
pub(crate) fn shared() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("ts-music/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    })
}

// Browser-like identity for public lyric endpoints that are picky about it
// (LRCLIB). NetEase and Musixmatch requests already carry their own explicit
// per-request User-Agent headers and ignore this one.
pub(crate) const LYRICS_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// Cover-art lookups stay short so Discord presence updates never stall on a
// slow lookup chain (iTunes → Deezer fallbacks).
pub(crate) const COVER_TIMEOUT: Duration = Duration::from_secs(8);

// MusicBrainz asks clients for generous timeouts; the importer runs in bulk
// with progress events, so a slower worst case is fine there.
pub(crate) const IMPORT_TIMEOUT: Duration = Duration::from_secs(20);
