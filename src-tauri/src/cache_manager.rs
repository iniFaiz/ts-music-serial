//! Quota-bound native cache storage shared by covers, waveforms, lyrics and
//! loudness analysis. Cache artifacts are written through a temporary sibling,
//! indexed with an LRU timestamp and garbage-collected independently per kind.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheKind {
    Covers,
    Waveforms,
    Lyrics,
    Loudness,
}

impl CacheKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Covers => "covers",
            Self::Waveforms => "waveforms",
            Self::Lyrics => "lyrics",
            Self::Loudness => "loudness",
        }
    }

    fn quota(self) -> u64 {
        match self {
            Self::Covers => 256 * 1024 * 1024,
            Self::Waveforms => 64 * 1024 * 1024,
            Self::Lyrics => 32 * 1024 * 1024,
            Self::Loudness => 16 * 1024 * 1024,
        }
    }

    fn all() -> [Self; 4] {
        [Self::Covers, Self::Waveforms, Self::Lyrics, Self::Loudness]
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "covers" => Some(Self::Covers),
            "waveforms" => Some(Self::Waveforms),
            "lyrics" => Some(Self::Lyrics),
            "loudness" => Some(Self::Loudness),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheEntry {
    size: u64,
    last_access_ms: u64,
    source_path: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct CacheIndex {
    entries: HashMap<String, CacheEntry>,
}

struct CacheInner {
    root: PathBuf,
    index: Mutex<CacheIndex>,
    nonce: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct CacheManager {
    inner: Arc<CacheInner>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CacheCleanup {
    removed_files: u64,
    bytes_freed: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl CacheManager {
    pub(crate) fn new(app: &AppHandle) -> Result<Self, String> {
        let root = app
            .path()
            .app_cache_dir()
            .map_err(|error| format!("Cache directory unavailable: {error}"))?;
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        for kind in CacheKind::all() {
            fs::create_dir_all(root.join(kind.directory())).map_err(|error| error.to_string())?;
        }
        let index = fs::read(root.join("cache-index.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Ok(Self {
            inner: Arc::new(CacheInner {
                root,
                index: Mutex::new(index),
                nonce: AtomicU64::new(0),
            }),
        })
    }

    pub(crate) fn directory(&self, kind: CacheKind) -> PathBuf {
        self.inner.root.join(kind.directory())
    }

    fn relative_key(kind: CacheKind, name: &str) -> Result<String, String> {
        let path = Path::new(name);
        if path.file_name().and_then(|value| value.to_str()) != Some(name) {
            return Err("Invalid cache file name".to_string());
        }
        Ok(format!("{}/{}", kind.directory(), name))
    }

    fn persist_index(&self, index: &CacheIndex) -> Result<(), String> {
        let bytes = serde_json::to_vec(index).map_err(|error| error.to_string())?;
        self.atomic_replace(&self.inner.root.join("cache-index.json"), &bytes)
    }

    fn atomic_replace(&self, target: &Path, bytes: &[u8]) -> Result<(), String> {
        let nonce = self.inner.nonce.fetch_add(1, Ordering::Relaxed);
        let temp = target.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temp);
            return Err(error.to_string());
        }
        drop(file);
        if target.exists() {
            let _ = fs::remove_file(target);
        }
        fs::rename(&temp, target).map_err(|error| {
            let _ = fs::remove_file(&temp);
            error.to_string()
        })
    }

    pub(crate) fn read(&self, kind: CacheKind, name: &str) -> Option<Vec<u8>> {
        let relative = Self::relative_key(kind, name).ok()?;
        let bytes = fs::read(self.directory(kind).join(name)).ok()?;
        let mut index = self.inner.index.lock();
        let source_path = index
            .entries
            .get(&relative)
            .and_then(|entry| entry.source_path.clone());
        index.entries.insert(
            relative,
            CacheEntry {
                size: bytes.len() as u64,
                last_access_ms: now_ms(),
                source_path,
            },
        );
        Some(bytes)
    }

    pub(crate) fn cached_path(&self, kind: CacheKind, name: &str) -> Option<PathBuf> {
        let relative = Self::relative_key(kind, name).ok()?;
        let path = self.directory(kind).join(name);
        let size = fs::metadata(&path).ok()?.len();
        let mut index = self.inner.index.lock();
        let source_path = index
            .entries
            .get(&relative)
            .and_then(|entry| entry.source_path.clone());
        index.entries.insert(
            relative,
            CacheEntry {
                size,
                last_access_ms: now_ms(),
                source_path,
            },
        );
        Some(path)
    }

    pub(crate) fn write(
        &self,
        kind: CacheKind,
        name: &str,
        bytes: &[u8],
        source_path: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let relative = Self::relative_key(kind, name)?;
        let target = self.directory(kind).join(name);
        self.atomic_replace(&target, bytes)?;
        let source = source_path.map(|path| path.to_string_lossy().into_owned());
        let extension = target.extension().map(|value| value.to_os_string());
        let mut index = self.inner.index.lock();

        // A changed mtime produces a new cache key. Retire the older artifact
        // for the same source and artifact type immediately.
        if let Some(source) = source.as_ref() {
            let stale: Vec<String> = index
                .entries
                .iter()
                .filter_map(|(key, entry)| {
                    let same_extension = extension.as_ref().is_some_and(|wanted| {
                        Path::new(key)
                            .extension()
                            .is_some_and(|found| found == wanted)
                    });
                    (key != &relative
                        && key.starts_with(kind.directory())
                        && entry.source_path.as_ref() == Some(source)
                        && same_extension)
                        .then(|| key.clone())
                })
                .collect();
            for key in stale {
                let _ = fs::remove_file(self.inner.root.join(&key));
                index.entries.remove(&key);
            }
        }

        index.entries.insert(
            relative,
            CacheEntry {
                size: bytes.len() as u64,
                last_access_ms: now_ms(),
                source_path: source,
            },
        );
        self.enforce_quota_locked(kind, &mut index);
        self.persist_index(&index)?;
        Ok(target)
    }

    fn enforce_quota_locked(&self, kind: CacheKind, index: &mut CacheIndex) {
        let prefix = format!("{}/", kind.directory());
        let mut entries: Vec<(String, CacheEntry)> = index
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect();
        let mut total: u64 = entries.iter().map(|(_, entry)| entry.size).sum();
        entries.sort_by_key(|(_, entry)| entry.last_access_ms);
        for (key, entry) in entries {
            if total <= kind.quota() {
                break;
            }
            if fs::remove_file(self.inner.root.join(&key)).is_ok() {
                total = total.saturating_sub(entry.size);
            }
            index.entries.remove(&key);
        }
    }

    pub(crate) fn invalidate_source(&self, source_path: &Path) {
        let source = source_path.to_string_lossy();
        let mut index = self.inner.index.lock();
        let stale: Vec<String> = index
            .entries
            .iter()
            .filter(|(_, entry)| entry.source_path.as_deref() == Some(source.as_ref()))
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            let _ = fs::remove_file(self.inner.root.join(&key));
            index.entries.remove(&key);
        }
        let _ = self.persist_index(&index);
    }

    pub(crate) fn gc(&self) -> CacheCleanup {
        let mut index = self.inner.index.lock();
        let mut removed_files = 0;
        let mut bytes_freed = 0;

        index.entries.retain(|relative, entry| {
            let exists = self.inner.root.join(relative).is_file();
            let source_exists = entry
                .source_path
                .as_ref()
                .is_none_or(|source| Path::new(source).is_file());
            if exists && source_exists {
                true
            } else {
                if exists && fs::remove_file(self.inner.root.join(relative)).is_ok() {
                    removed_files += 1;
                    bytes_freed += entry.size;
                }
                false
            }
        });

        // Discover cache files from older app versions so quotas also cover
        // artifacts created before the LRU index existed.
        for kind in CacheKind::all() {
            if let Ok(files) = fs::read_dir(self.directory(kind)) {
                for file in files.flatten() {
                    let path = file.path();
                    if !path.is_file()
                        || path.extension().is_some_and(|extension| {
                            extension.to_string_lossy().starts_with("tmp-")
                        })
                    {
                        continue;
                    }
                    let relative = format!(
                        "{}/{}",
                        kind.directory(),
                        file.file_name().to_string_lossy()
                    );
                    index.entries.entry(relative).or_insert_with(|| CacheEntry {
                        size: file.metadata().map(|metadata| metadata.len()).unwrap_or(0),
                        last_access_ms: file
                            .metadata()
                            .ok()
                            .and_then(|metadata| metadata.modified().ok())
                            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                            .map(|duration| duration.as_millis() as u64)
                            .unwrap_or_else(now_ms),
                        source_path: None,
                    });
                }
            }
            let before: u64 = index
                .entries
                .iter()
                .filter(|(key, _)| key.starts_with(kind.directory()))
                .map(|(_, entry)| entry.size)
                .sum();
            self.enforce_quota_locked(kind, &mut index);
            let after: u64 = index
                .entries
                .iter()
                .filter(|(key, _)| key.starts_with(kind.directory()))
                .map(|(_, entry)| entry.size)
                .sum();
            bytes_freed += before.saturating_sub(after);
        }
        let _ = self.persist_index(&index);
        CacheCleanup {
            removed_files,
            bytes_freed,
        }
    }

    pub(crate) fn clear(&self, kind: Option<CacheKind>) -> CacheCleanup {
        let mut index = self.inner.index.lock();
        let kinds: Vec<CacheKind> = kind.map_or_else(|| CacheKind::all().to_vec(), |k| vec![k]);
        let mut removed_files = 0;
        let mut bytes_freed = 0;
        for current in kinds {
            if let Ok(files) = fs::read_dir(self.directory(current)) {
                for file in files.flatten() {
                    if let Ok(metadata) = file.metadata() {
                        if metadata.is_file() && fs::remove_file(file.path()).is_ok() {
                            removed_files += 1;
                            bytes_freed += metadata.len();
                        }
                    }
                }
            }
            let prefix = format!("{}/", current.directory());
            index.entries.retain(|key, _| !key.starts_with(&prefix));
        }
        let _ = self.persist_index(&index);
        CacheCleanup {
            removed_files,
            bytes_freed,
        }
    }
}

pub(crate) fn manager(app: &AppHandle) -> Option<CacheManager> {
    app.try_state::<CacheManager>()
        .map(|state| state.inner().clone())
}

#[tauri::command]
pub(crate) fn clear_cache(app: AppHandle, kind: Option<String>) -> Result<CacheCleanup, String> {
    let kind = match kind.as_deref() {
        Some(value) => {
            Some(CacheKind::parse(value).ok_or_else(|| "Unknown cache kind".to_string())?)
        }
        None => None,
    };
    let manager = manager(&app).ok_or_else(|| "Cache manager unavailable".to_string())?;
    Ok(manager.clear(kind))
}
