//! Queue and transition policy for native playback.
//!
//! The audio engine deliberately remains separate from this module: this is a
//! deterministic state machine which decides *what* should play, while
//! `player::mod` owns decoders and output devices.  Keeping the policy pure
//! makes queue races testable and gives the webview a typed intent/snapshot
//! boundary during the gradual migration away from frontend-owned playback.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const MAX_HISTORY: usize = 256;
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueEntry {
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) duration_hint: f64,
    #[serde(default)]
    pub(crate) track_gain_db: Option<f64>,
    #[serde(default)]
    pub(crate) track_peak: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransitionMode {
    #[default]
    Off,
    Gapless,
    Crossfade,
}

impl TransitionMode {
    pub(crate) fn from_legacy(value: &str) -> Self {
        match value {
            "gapless" => Self::Gapless,
            "crossfade" => Self::Crossfade,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SleepMode {
    #[default]
    Off,
    EndTrack,
    EndQueue,
    Deadline {
        deadline_ms: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedToken {
    pub(crate) entry_id: String,
    pub(crate) path: String,
    pub(crate) generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlaybackSessionSnapshot {
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) queue: Vec<QueueEntry>,
    pub(crate) current_entry_id: Option<String>,
    pub(crate) next_entry_id: Option<String>,
    pub(crate) history: Vec<String>,
    pub(crate) shuffle: bool,
    pub(crate) repeat: RepeatMode,
    pub(crate) autoplay: bool,
    pub(crate) playing: bool,
    pub(crate) transition: TransitionMode,
    pub(crate) crossfade_secs: f64,
    pub(crate) sleep: SleepMode,
    pub(crate) prepared: Option<PreparedToken>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PlaybackEffect {
    Load {
        entry: QueueEntry,
        autoplay: bool,
        start_at: Option<f64>,
    },
    SetPlaying {
        playing: bool,
    },
    Stop {
        reason: StopReason,
    },
    RequestAutoplay,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopReason {
    QueueEnded,
    SleepEndTrack,
    SleepEndQueue,
    SleepDeadline,
    QueueEmpty,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountingKind {
    PlayStarted,
    PlayCounted,
    SkipCounted,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PlaybackSessionEvent {
    QueueChanged,
    CurrentChanged {
        entry: Option<QueueEntry>,
        reason: ChangeReason,
    },
    ModesChanged,
    TransitionChanged,
    SleepChanged,
    PreparedInvalidated {
        generation: u64,
    },
    Accounting {
        kind: AccountingKind,
        entry_id: String,
        path: String,
        position: f64,
    },
    AutoplayRequested,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangeReason {
    PlayQueue,
    Select,
    Next,
    Previous,
    AutomaticTransition,
    Removed,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlaybackSessionUpdate {
    pub(crate) snapshot: PlaybackSessionSnapshot,
    pub(crate) events: Vec<PlaybackSessionEvent>,
    pub(crate) effect: Option<PlaybackEffect>,
    /// The audio layer uses this bit to drop a decoder immediately.  Exposing it
    /// also lets a future frontend explain why a preparation was cancelled.
    pub(crate) prepared_invalidated: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PlaybackIntent {
    PlayQueue {
        entries: Vec<QueueEntry>,
        start_entry_id: Option<String>,
        #[serde(default = "default_true")]
        autoplay: bool,
        #[serde(default)]
        start_at: Option<f64>,
        #[serde(default)]
        shuffle: Option<bool>,
        #[serde(default)]
        repeat: Option<RepeatMode>,
        #[serde(default)]
        autoplay_mode: Option<bool>,
        #[serde(default)]
        sleep: Option<SleepMode>,
    },
    SelectEntry {
        entry_id: String,
        #[serde(default = "default_true")]
        autoplay: bool,
        #[serde(default)]
        start_at: Option<f64>,
    },
    Enqueue {
        entries: Vec<QueueEntry>,
        #[serde(default)]
        after_current: bool,
    },
    Next {
        #[serde(default)]
        user_triggered: bool,
    },
    Previous {
        #[serde(default)]
        position: f64,
    },
    MoveQueueItem {
        entry_id: String,
        to_index: usize,
    },
    RemoveQueueItem {
        entry_id: String,
    },
    ClearUpcoming,
    AppendAutoplay {
        entry: QueueEntry,
        #[serde(default = "default_true")]
        play_now: bool,
    },
    SetModes {
        shuffle: bool,
        repeat: RepeatMode,
        autoplay: bool,
    },
    SetTransition {
        mode: TransitionMode,
        crossfade_secs: f64,
    },
    SetSleep {
        sleep: SleepMode,
    },
    ObserveProgress {
        position: f64,
        duration: f64,
        #[serde(default)]
        finished: bool,
    },
    SetPlaying {
        playing: bool,
    },
    Clear,
}

impl PlaybackIntent {
    pub(crate) fn validate_and_authorize<R: tauri::Runtime>(
        mut self,
        app: &tauri::AppHandle<R>,
    ) -> Result<Self, String> {
        fn validate_position(value: f64, field: &str) -> Result<(), String> {
            if !value.is_finite() || !(0.0..=7.0 * 24.0 * 60.0 * 60.0).contains(&value) {
                return Err(format!("{field} is outside the supported range"));
            }
            Ok(())
        }

        fn authorize_entries<R: tauri::Runtime>(
            app: &tauri::AppHandle<R>,
            entries: &mut [QueueEntry],
        ) -> Result<(), String> {
            if entries.len() > crate::limits::MAX_QUEUE_ENTRIES {
                return Err(format!(
                    "Queue contains too many entries (max {})",
                    crate::limits::MAX_QUEUE_ENTRIES
                ));
            }
            for entry in entries {
                crate::limits::validate_text(&entry.id, "Queue entry ID", 128)?;
                crate::limits::validate_text(
                    &entry.path,
                    "Queue path",
                    crate::limits::MAX_PATH_BYTES,
                )?;
                validate_position(entry.duration_hint, "Queue duration")?;
                if entry
                    .track_gain_db
                    .is_some_and(|value| !value.is_finite() || !(-100.0..=100.0).contains(&value))
                {
                    return Err("Queue gain is outside the supported range".to_string());
                }
                if entry
                    .track_peak
                    .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
                {
                    return Err("Queue peak is outside the supported range".to_string());
                }
                entry.path = crate::resolve_allowed_audio(app, std::path::Path::new(&entry.path))?
                    .to_string_lossy()
                    .to_string();
            }
            Ok(())
        }

        match &mut self {
            Self::PlayQueue {
                entries,
                start_entry_id,
                start_at,
                ..
            } => {
                authorize_entries(app, entries)?;
                if let Some(entry_id) = start_entry_id {
                    crate::limits::validate_text(entry_id, "Queue entry ID", 128)?;
                }
                if let Some(position) = start_at {
                    validate_position(*position, "Start position")?;
                }
            }
            Self::Enqueue { entries, .. } => authorize_entries(app, entries)?,
            Self::AppendAutoplay { entry, .. } => {
                authorize_entries(app, std::slice::from_mut(entry))?
            }
            Self::SelectEntry {
                entry_id, start_at, ..
            } => {
                crate::limits::validate_text(entry_id, "Queue entry ID", 128)?;
                if let Some(position) = start_at {
                    validate_position(*position, "Start position")?;
                }
            }
            Self::MoveQueueItem { entry_id, to_index } => {
                crate::limits::validate_text(entry_id, "Queue entry ID", 128)?;
                if *to_index >= crate::limits::MAX_QUEUE_ENTRIES {
                    return Err("Queue destination is outside the supported range".to_string());
                }
            }
            Self::RemoveQueueItem { entry_id } => {
                crate::limits::validate_text(entry_id, "Queue entry ID", 128)?;
            }
            Self::Previous { position } => validate_position(*position, "Playback position")?,
            Self::SetTransition { crossfade_secs, .. } => {
                if !crossfade_secs.is_finite() || !(0.0..=60.0).contains(crossfade_secs) {
                    return Err("Crossfade duration is outside the supported range".to_string());
                }
            }
            Self::ObserveProgress {
                position, duration, ..
            } => {
                validate_position(*position, "Playback position")?;
                validate_position(*duration, "Track duration")?;
            }
            Self::Next { .. }
            | Self::ClearUpcoming
            | Self::SetModes { .. }
            | Self::SetSleep { .. }
            | Self::SetPlaying { .. }
            | Self::Clear => {}
        }
        Ok(self)
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default)]
struct AccountingState {
    entry_id: Option<String>,
    position: f64,
    duration: f64,
    play_started: bool,
    play_counted: bool,
    finished: bool,
}

#[derive(Debug)]
struct SessionState {
    session_id: u64,
    entry_sequence: u64,
    revision: u64,
    generation: u64,
    random_state: u64,
    queue: Vec<QueueEntry>,
    current_entry_id: Option<String>,
    next_entry_id: Option<String>,
    history: Vec<String>,
    shuffle: bool,
    repeat: RepeatMode,
    autoplay: bool,
    playing: bool,
    transition: TransitionMode,
    crossfade_secs: f64,
    sleep: SleepMode,
    prepared: Option<PreparedToken>,
    accounting: AccountingState,
}

impl Default for SessionState {
    fn default() -> Self {
        let wall_clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|v| v.as_nanos() as u64)
            .unwrap_or(0);
        let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let session_id = wall_clock ^ sequence.rotate_left(17);
        Self {
            session_id,
            entry_sequence: 0,
            revision: 0,
            generation: 1,
            random_state: session_id | 1,
            queue: Vec::new(),
            current_entry_id: None,
            next_entry_id: None,
            history: Vec::new(),
            shuffle: false,
            repeat: RepeatMode::Off,
            autoplay: false,
            playing: false,
            transition: TransitionMode::Off,
            crossfade_secs: 6.0,
            sleep: SleepMode::Off,
            prepared: None,
            accounting: AccountingState::default(),
        }
    }
}

impl SessionState {
    fn snapshot(&self) -> PlaybackSessionSnapshot {
        PlaybackSessionSnapshot {
            revision: self.revision,
            generation: self.generation,
            queue: self.queue.clone(),
            current_entry_id: self.current_entry_id.clone(),
            next_entry_id: self.next_entry_id.clone(),
            history: self.history.clone(),
            shuffle: self.shuffle,
            repeat: self.repeat,
            autoplay: self.autoplay,
            playing: self.playing,
            transition: self.transition,
            crossfade_secs: self.crossfade_secs,
            sleep: self.sleep.clone(),
            prepared: self.prepared.clone(),
        }
    }

    fn update(
        &self,
        events: Vec<PlaybackSessionEvent>,
        effect: Option<PlaybackEffect>,
        prepared_invalidated: bool,
    ) -> PlaybackSessionUpdate {
        PlaybackSessionUpdate {
            snapshot: self.snapshot(),
            events,
            effect,
            prepared_invalidated,
        }
    }

    fn next_generated_id(&mut self) -> String {
        self.entry_sequence = self.entry_sequence.wrapping_add(1);
        format!("q-{:016x}-{:016x}", self.session_id, self.entry_sequence)
    }

    fn normalize_entries(&mut self, entries: Vec<QueueEntry>) -> Result<Vec<QueueEntry>, String> {
        let mut seen = HashSet::with_capacity(entries.len());
        let mut normalized = Vec::with_capacity(entries.len());
        for mut entry in entries {
            if entry.path.trim().is_empty() {
                return Err("Queue entries must have a non-empty path".to_string());
            }
            if entry.id.trim().is_empty() || !seen.insert(entry.id.clone()) {
                loop {
                    entry.id = self.next_generated_id();
                    if seen.insert(entry.id.clone()) {
                        break;
                    }
                }
            }
            entry.duration_hint = entry.duration_hint.max(0.0);
            normalized.push(entry);
        }
        Ok(normalized)
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.queue.iter().position(|entry| entry.id == id)
    }

    fn entry(&self, id: &str) -> Option<QueueEntry> {
        self.queue.iter().find(|entry| entry.id == id).cloned()
    }

    fn bump_policy_generation(&mut self) -> bool {
        self.revision = self.revision.wrapping_add(1);
        self.generation = self.generation.wrapping_add(1).max(1);
        let invalidated = self.prepared.take().is_some();
        self.next_entry_id = None;
        invalidated
    }

    fn next_random(&mut self, upper: usize) -> usize {
        // Xorshift64*: cheap, deterministic, and sufficient for queue order.
        let mut x = self.random_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.random_state = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) % upper as u64) as usize
    }

    fn plan_next(&mut self) {
        self.next_entry_id = None;
        let Some(current_id) = self.current_entry_id.clone() else {
            return;
        };
        if matches!(self.sleep, SleepMode::EndTrack) {
            return;
        }
        if self.repeat == RepeatMode::One {
            self.next_entry_id = Some(current_id);
            return;
        }
        let Some(current_index) = self.index_of(&current_id) else {
            return;
        };
        if self.shuffle {
            let candidates: Vec<String> = self
                .queue
                .iter()
                .filter(|entry| entry.id != current_id)
                .map(|entry| entry.id.clone())
                .collect();
            if !candidates.is_empty() {
                let index = self.next_random(candidates.len());
                self.next_entry_id = Some(candidates[index].clone());
            } else if self.repeat == RepeatMode::All {
                self.next_entry_id = Some(current_id);
            }
            return;
        }
        if let Some(entry) = self.queue.get(current_index + 1) {
            self.next_entry_id = Some(entry.id.clone());
        } else if self.repeat == RepeatMode::All && !self.queue.is_empty() {
            self.next_entry_id = Some(self.queue[0].id.clone());
        }
    }

    fn push_history(&mut self, id: String) {
        if self.history.last() != Some(&id) {
            self.history.push(id);
        }
        if self.history.len() > MAX_HISTORY {
            self.history.drain(..self.history.len() - MAX_HISTORY);
        }
    }

    fn accounting_events_for_departure(&mut self, natural: bool) -> Vec<PlaybackSessionEvent> {
        let mut events = Vec::new();
        let accounting = std::mem::take(&mut self.accounting);
        let Some(entry_id) = accounting.entry_id else {
            return events;
        };
        if !natural && !accounting.finished && !accounting.play_counted && accounting.position > 2.0
        {
            if let Some(entry) = self.entry(&entry_id) {
                events.push(PlaybackSessionEvent::Accounting {
                    kind: AccountingKind::SkipCounted,
                    entry_id,
                    path: entry.path,
                    position: accounting.position,
                });
            }
        }
        events
    }

    fn start_accounting(&mut self, entry_id: &str) -> Option<PlaybackSessionEvent> {
        self.accounting = AccountingState {
            entry_id: Some(entry_id.to_string()),
            play_started: self.playing,
            ..AccountingState::default()
        };
        if !self.playing {
            return None;
        }
        self.entry(entry_id)
            .map(|entry| PlaybackSessionEvent::Accounting {
                kind: AccountingKind::PlayStarted,
                entry_id: entry.id,
                path: entry.path,
                position: 0.0,
            })
    }

    fn deadline_expired(&self) -> bool {
        let SleepMode::Deadline { deadline_ms } = self.sleep else {
            return false;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis() as u64)
            .unwrap_or(0);
        now >= deadline_ms
    }

    fn set_current(
        &mut self,
        entry_id: Option<String>,
        reason: ChangeReason,
        natural_departure: bool,
    ) -> Vec<PlaybackSessionEvent> {
        if self.current_entry_id == entry_id {
            return Vec::new();
        }
        let mut events = self.accounting_events_for_departure(natural_departure);
        if let Some(old) = self.current_entry_id.take() {
            self.push_history(old);
        }
        self.current_entry_id = entry_id.clone();
        if let Some(id) = entry_id.as_deref() {
            if let Some(started) = self.start_accounting(id) {
                events.push(started);
            }
        }
        events.push(PlaybackSessionEvent::CurrentChanged {
            entry: entry_id.as_deref().and_then(|id| self.entry(id)),
            reason,
        });
        events
    }

    fn stop(
        &mut self,
        reason: StopReason,
        mut events: Vec<PlaybackSessionEvent>,
    ) -> (Vec<PlaybackSessionEvent>, PlaybackEffect) {
        self.playing = false;
        if matches!(
            reason,
            StopReason::SleepEndTrack | StopReason::SleepEndQueue | StopReason::SleepDeadline
        ) {
            self.sleep = SleepMode::Off;
            events.push(PlaybackSessionEvent::SleepChanged);
        }
        (events, PlaybackEffect::Stop { reason })
    }
}

/// Cloneable handle embedded in `AudioPlayer`. Queue policy and playback state
/// are mutated only through this state machine; the webview receives snapshots.
#[derive(Clone, Debug, Default)]
pub(crate) struct PlaybackSession {
    inner: Arc<Mutex<SessionState>>,
}

impl PlaybackSession {
    pub(crate) fn snapshot(&self) -> PlaybackSessionSnapshot {
        self.inner.lock().snapshot()
    }

    pub(crate) fn snapshot_update(&self) -> PlaybackSessionUpdate {
        self.inner.lock().update(Vec::new(), None, false)
    }

    /// Restore only durable queue policy. Playback always resumes paused and
    /// decoder/preparation/accounting state is intentionally rebuilt fresh.
    pub(crate) fn restore(&self, snapshot: PlaybackSessionSnapshot) -> Result<(), String> {
        if snapshot.queue.len() > crate::limits::MAX_QUEUE_ENTRIES {
            return Err(format!(
                "Persisted queue contains too many entries (max {})",
                crate::limits::MAX_QUEUE_ENTRIES
            ));
        }
        if !snapshot.crossfade_secs.is_finite() || !(0.0..=60.0).contains(&snapshot.crossfade_secs)
        {
            return Err("Persisted crossfade duration is invalid".to_string());
        }
        let mut restored = SessionState::default();
        restored.queue = restored.normalize_entries(snapshot.queue)?;
        let valid_ids = restored
            .queue
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<HashSet<_>>();
        restored.current_entry_id = snapshot
            .current_entry_id
            .filter(|id| valid_ids.contains(id));
        restored.history = snapshot
            .history
            .into_iter()
            .filter(|id| valid_ids.contains(id))
            .take(MAX_HISTORY)
            .collect();
        restored.shuffle = snapshot.shuffle;
        restored.repeat = snapshot.repeat;
        restored.autoplay = snapshot.autoplay;
        restored.playing = false;
        restored.transition = snapshot.transition;
        restored.crossfade_secs = snapshot.crossfade_secs;
        restored.sleep = match snapshot.sleep {
            SleepMode::Deadline { deadline_ms }
                if SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|value| value.as_millis() as u64 >= deadline_ms)
                    .unwrap_or(true) =>
            {
                SleepMode::Off
            }
            sleep => sleep,
        };
        restored.revision = snapshot.revision.wrapping_add(1);
        restored.generation = snapshot.generation.wrapping_add(1).max(1);
        restored.plan_next();
        *self.inner.lock() = restored;
        Ok(())
    }

    pub(crate) fn apply(&self, intent: PlaybackIntent) -> Result<PlaybackSessionUpdate, String> {
        let mut state = self.inner.lock();
        let mut events = Vec::new();
        let mut effect = None;
        let mut invalidated = false;

        match intent {
            PlaybackIntent::PlayQueue {
                entries,
                start_entry_id,
                autoplay,
                start_at,
                shuffle,
                repeat,
                autoplay_mode,
                sleep,
            } => {
                let queue = state.normalize_entries(entries)?;
                if queue.is_empty() {
                    invalidated |= state.bump_policy_generation();
                    state.queue.clear();
                    state.current_entry_id = None;
                    state.playing = false;
                    events.push(PlaybackSessionEvent::QueueChanged);
                    effect = Some(PlaybackEffect::Stop {
                        reason: StopReason::QueueEmpty,
                    });
                } else {
                    let selected = start_entry_id
                        .filter(|id| queue.iter().any(|entry| &entry.id == id))
                        .unwrap_or_else(|| queue[0].id.clone());
                    invalidated |= state.bump_policy_generation();
                    state.queue = queue;
                    state.playing = autoplay;
                    if let Some(value) = shuffle {
                        state.shuffle = value;
                    }
                    if let Some(value) = repeat {
                        state.repeat = value;
                    }
                    if let Some(value) = autoplay_mode {
                        state.autoplay = value;
                    }
                    if let Some(value) = sleep {
                        state.sleep = value;
                    }
                    events.push(PlaybackSessionEvent::QueueChanged);
                    events.push(PlaybackSessionEvent::ModesChanged);
                    events.push(PlaybackSessionEvent::SleepChanged);
                    events.extend(state.set_current(
                        Some(selected.clone()),
                        ChangeReason::PlayQueue,
                        false,
                    ));
                    state.plan_next();
                    effect = state.entry(&selected).map(|entry| PlaybackEffect::Load {
                        entry,
                        autoplay,
                        start_at: start_at.map(|position| position.max(0.0)),
                    });
                }
            }
            PlaybackIntent::SelectEntry {
                entry_id,
                autoplay,
                start_at,
            } => {
                let Some(entry) = state.entry(&entry_id) else {
                    return Err(format!("Queue entry does not exist: {entry_id}"));
                };
                invalidated |= state.bump_policy_generation();
                state.playing = autoplay;
                if state.current_entry_id.as_deref() == Some(entry_id.as_str()) {
                    events.extend(state.accounting_events_for_departure(false));
                    if let Some(started) = state.start_accounting(&entry_id) {
                        events.push(started);
                    }
                } else {
                    events.extend(state.set_current(Some(entry_id), ChangeReason::Select, false));
                }
                state.plan_next();
                effect = Some(PlaybackEffect::Load {
                    entry,
                    autoplay,
                    start_at: start_at.map(|position| position.max(0.0)),
                });
            }
            PlaybackIntent::Enqueue {
                entries,
                after_current,
            } => {
                if state.queue.len().saturating_add(entries.len())
                    > crate::limits::MAX_QUEUE_ENTRIES
                {
                    return Err(format!(
                        "Queue contains too many entries (max {})",
                        crate::limits::MAX_QUEUE_ENTRIES
                    ));
                }
                let mut additions = state.normalize_entries(entries)?;
                let mut existing: HashSet<String> =
                    state.queue.iter().map(|entry| entry.id.clone()).collect();
                for entry in &mut additions {
                    if existing.contains(&entry.id) {
                        loop {
                            entry.id = state.next_generated_id();
                            if existing.insert(entry.id.clone()) {
                                break;
                            }
                        }
                    } else {
                        existing.insert(entry.id.clone());
                    }
                }
                if !additions.is_empty() {
                    invalidated |= state.bump_policy_generation();
                    let index = if after_current {
                        state
                            .current_entry_id
                            .as_deref()
                            .and_then(|id| state.index_of(id))
                            .map(|index| index + 1)
                            .unwrap_or(state.queue.len())
                    } else {
                        state.queue.len()
                    };
                    state.queue.splice(index..index, additions);
                    state.plan_next();
                    events.push(PlaybackSessionEvent::QueueChanged);
                }
            }
            PlaybackIntent::Next { user_triggered } => {
                if state.current_entry_id.is_none() || state.queue.is_empty() {
                    let (ev, eff) = state.stop(StopReason::QueueEmpty, events);
                    events = ev;
                    effect = Some(eff);
                } else if state.deadline_expired() {
                    let (ev, eff) = state.stop(StopReason::SleepDeadline, events);
                    events = ev;
                    effect = Some(eff);
                } else if !user_triggered && matches!(state.sleep, SleepMode::EndTrack) {
                    let (ev, eff) = state.stop(StopReason::SleepEndTrack, events);
                    events = ev;
                    effect = Some(eff);
                } else {
                    let repeat_current = !user_triggered && state.repeat == RepeatMode::One;
                    let next_id = if repeat_current {
                        state.current_entry_id.clone()
                    } else {
                        state.next_entry_id.clone()
                    };
                    if let Some(next_id) = next_id {
                        invalidated |= state.bump_policy_generation();
                        state.playing = true;
                        if repeat_current {
                            state.accounting = AccountingState::default();
                            if let Some(started) = state.start_accounting(&next_id) {
                                events.push(started);
                            }
                        } else {
                            events.extend(state.set_current(
                                Some(next_id.clone()),
                                ChangeReason::Next,
                                !user_triggered,
                            ));
                        }
                        state.plan_next();
                        effect = state.entry(&next_id).map(|entry| PlaybackEffect::Load {
                            entry,
                            autoplay: true,
                            start_at: None,
                        });
                    } else if !user_triggered && matches!(state.sleep, SleepMode::EndQueue) {
                        let (ev, eff) = state.stop(StopReason::SleepEndQueue, events);
                        events = ev;
                        effect = Some(eff);
                    } else if state.autoplay {
                        events.push(PlaybackSessionEvent::AutoplayRequested);
                        effect = Some(PlaybackEffect::RequestAutoplay);
                    } else if user_triggered && !state.queue.is_empty() {
                        let first_id = state.queue[0].id.clone();
                        invalidated |= state.bump_policy_generation();
                        state.playing = true;
                        events.extend(state.set_current(
                            Some(first_id.clone()),
                            ChangeReason::Next,
                            false,
                        ));
                        state.plan_next();
                        effect = state.entry(&first_id).map(|entry| PlaybackEffect::Load {
                            entry,
                            autoplay: true,
                            start_at: None,
                        });
                    } else {
                        let (ev, eff) = state.stop(StopReason::QueueEnded, events);
                        events = ev;
                        effect = Some(eff);
                    }
                }
            }
            PlaybackIntent::Previous { position } => {
                if let Some(current) = state.current_entry_id.clone() {
                    let target = if position > 3.0 {
                        Some(current)
                    } else if state.shuffle {
                        state
                            .history
                            .pop()
                            .or_else(|| state.current_entry_id.clone())
                    } else {
                        let index = state.index_of(&current).unwrap_or(0);
                        if index > 0 {
                            Some(state.queue[index - 1].id.clone())
                        } else if state.repeat == RepeatMode::All && !state.queue.is_empty() {
                            state.queue.last().map(|entry| entry.id.clone())
                        } else {
                            Some(current)
                        }
                    };
                    if let Some(target) = target {
                        invalidated |= state.bump_policy_generation();
                        state.playing = true;
                        events.extend(state.set_current(
                            Some(target.clone()),
                            ChangeReason::Previous,
                            false,
                        ));
                        state.plan_next();
                        effect = state.entry(&target).map(|entry| PlaybackEffect::Load {
                            entry,
                            autoplay: true,
                            start_at: Some(0.0),
                        });
                    }
                }
            }
            PlaybackIntent::MoveQueueItem { entry_id, to_index } => {
                let from = state
                    .index_of(&entry_id)
                    .ok_or_else(|| "Queue entry not found".to_string())?;
                if state.queue.is_empty() || to_index >= state.queue.len() {
                    return Err("Queue destination is out of bounds".to_string());
                }
                if from != to_index {
                    let item = state.queue.remove(from);
                    state.queue.insert(to_index, item);
                    invalidated |= state.bump_policy_generation();
                    state.plan_next();
                    events.push(PlaybackSessionEvent::QueueChanged);
                }
            }
            PlaybackIntent::RemoveQueueItem { entry_id } => {
                let index = state
                    .index_of(&entry_id)
                    .ok_or_else(|| "Queue entry not found".to_string())?;
                let removed_current = state.current_entry_id.as_deref() == Some(&entry_id);
                state.queue.remove(index);
                invalidated |= state.bump_policy_generation();
                events.push(PlaybackSessionEvent::QueueChanged);
                if removed_current {
                    let replacement = state
                        .queue
                        .get(index.min(state.queue.len().saturating_sub(1)))
                        .map(|entry| entry.id.clone());
                    events.extend(state.set_current(
                        replacement.clone(),
                        ChangeReason::Removed,
                        false,
                    ));
                    effect = replacement
                        .as_deref()
                        .and_then(|id| state.entry(id))
                        .map(|entry| PlaybackEffect::Load {
                            entry,
                            autoplay: state.playing,
                            start_at: None,
                        })
                        .or(Some(PlaybackEffect::Stop {
                            reason: StopReason::QueueEmpty,
                        }));
                }
                state.plan_next();
            }
            PlaybackIntent::ClearUpcoming => {
                let retained = state
                    .current_entry_id
                    .as_deref()
                    .and_then(|id| state.entry(id))
                    .into_iter()
                    .collect::<Vec<_>>();
                if state.queue != retained {
                    invalidated |= state.bump_policy_generation();
                    state.queue = retained;
                    state.history.clear();
                    state.plan_next();
                    events.push(PlaybackSessionEvent::QueueChanged);
                }
            }
            PlaybackIntent::AppendAutoplay { entry, play_now } => {
                if state.queue.len() >= crate::limits::MAX_QUEUE_ENTRIES {
                    return Err(format!(
                        "Queue contains too many entries (max {})",
                        crate::limits::MAX_QUEUE_ENTRIES
                    ));
                }
                let mut normalized = state.normalize_entries(vec![entry])?;
                let entry = normalized.remove(0);
                let entry_id = entry.id.clone();
                state.queue.push(entry.clone());
                invalidated |= state.bump_policy_generation();
                events.push(PlaybackSessionEvent::QueueChanged);
                if play_now {
                    state.playing = true;
                    events.extend(state.set_current(Some(entry_id), ChangeReason::Next, true));
                    effect = Some(PlaybackEffect::Load {
                        entry,
                        autoplay: true,
                        start_at: None,
                    });
                }
                state.plan_next();
            }
            PlaybackIntent::SetModes {
                shuffle,
                repeat,
                autoplay,
            } => {
                if state.shuffle != shuffle || state.repeat != repeat || state.autoplay != autoplay
                {
                    state.shuffle = shuffle;
                    state.repeat = repeat;
                    state.autoplay = autoplay;
                    invalidated |= state.bump_policy_generation();
                    state.plan_next();
                    events.push(PlaybackSessionEvent::ModesChanged);
                }
            }
            PlaybackIntent::SetTransition {
                mode,
                crossfade_secs,
            } => {
                let seconds = crossfade_secs.clamp(0.25, 12.0);
                if state.transition != mode || (state.crossfade_secs - seconds).abs() > f64::EPSILON
                {
                    state.transition = mode;
                    state.crossfade_secs = seconds;
                    invalidated |= state.bump_policy_generation();
                    state.plan_next();
                    events.push(PlaybackSessionEvent::TransitionChanged);
                }
            }
            PlaybackIntent::SetSleep { sleep } => {
                if state.sleep != sleep {
                    state.sleep = sleep;
                    invalidated |= state.bump_policy_generation();
                    state.plan_next();
                    events.push(PlaybackSessionEvent::SleepChanged);
                }
            }
            PlaybackIntent::ObserveProgress {
                position,
                duration,
                finished,
            } => {
                state.accounting.position = position.max(0.0);
                state.accounting.duration = duration.max(0.0);
                state.accounting.finished = finished;
                let threshold = (state.accounting.duration * 0.5).min(240.0);
                if threshold > 0.0
                    && state.accounting.position >= threshold
                    && !state.accounting.play_counted
                {
                    state.accounting.play_counted = true;
                    if let Some(id) = state.accounting.entry_id.clone() {
                        if let Some(entry) = state.entry(&id) {
                            events.push(PlaybackSessionEvent::Accounting {
                                kind: AccountingKind::PlayCounted,
                                entry_id: id,
                                path: entry.path,
                                position: state.accounting.position,
                            });
                        }
                    }
                }
                if state.deadline_expired() {
                    let (ev, eff) = state.stop(StopReason::SleepDeadline, events);
                    events = ev;
                    effect = Some(eff);
                }
            }
            PlaybackIntent::SetPlaying { playing } => {
                if state.playing != playing {
                    state.playing = playing;
                    state.revision = state.revision.wrapping_add(1);
                    if playing && !state.accounting.play_started {
                        state.accounting.play_started = true;
                        if let Some(id) = state.current_entry_id.clone() {
                            if let Some(entry) = state.entry(&id) {
                                events.push(PlaybackSessionEvent::Accounting {
                                    kind: AccountingKind::PlayStarted,
                                    entry_id: id,
                                    path: entry.path,
                                    position: state.accounting.position,
                                });
                            }
                        }
                    }
                    effect = Some(PlaybackEffect::SetPlaying { playing });
                }
            }
            PlaybackIntent::Clear => {
                invalidated |= state.bump_policy_generation();
                events.extend(state.set_current(None, ChangeReason::Removed, false));
                state.queue.clear();
                state.history.clear();
                state.playing = false;
                events.push(PlaybackSessionEvent::QueueChanged);
                effect = Some(PlaybackEffect::Stop {
                    reason: StopReason::QueueEmpty,
                });
            }
        }

        if invalidated {
            events.push(PlaybackSessionEvent::PreparedInvalidated {
                generation: state.generation,
            });
        }
        Ok(state.update(events, effect, invalidated))
    }

    /// Registers a decoder request only for the exact next entry planned by the
    /// native session. A caller cannot prepare an arbitrary stale queue item.
    pub(crate) fn begin_prepare(
        &self,
        path: &str,
        queue_entry_id: Option<&str>,
    ) -> Option<PreparedToken> {
        let mut state = self.inner.lock();
        if state.transition == TransitionMode::Off {
            state.prepared = None;
            return None;
        }
        let id = state.next_entry_id.clone()?;
        if queue_entry_id.is_some_and(|requested| requested != id)
            || state.entry(&id).as_ref().map(|entry| entry.path.as_str()) != Some(path)
        {
            return None;
        }
        let token = PreparedToken {
            entry_id: id,
            path: path.to_string(),
            generation: state.generation,
        };
        state.prepared = Some(token.clone());
        Some(token)
    }

    pub(crate) fn is_prepare_current(&self, token: &PreparedToken) -> bool {
        let state = self.inner.lock();
        state.generation == token.generation && state.prepared.as_ref() == Some(token)
    }

    /// Called only after the audio engine has actually crossed a boundary.
    pub(crate) fn promote_prepared(&self, token: &PreparedToken) -> Option<PlaybackSessionUpdate> {
        let mut state = self.inner.lock();
        if state.generation != token.generation || state.prepared.as_ref() != Some(token) {
            return None;
        }
        if state
            .entry(&token.entry_id)
            .as_ref()
            .map(|entry| entry.path.as_str())
            != Some(token.path.as_str())
        {
            return None;
        }
        state.prepared = None;
        state.revision = state.revision.wrapping_add(1);
        state.generation = state.generation.wrapping_add(1).max(1);
        state.playing = true;
        let mut events = state.set_current(
            Some(token.entry_id.clone()),
            ChangeReason::AutomaticTransition,
            true,
        );
        state.plan_next();
        events.push(PlaybackSessionEvent::PreparedInvalidated {
            generation: state.generation,
        });
        Some(state.update(events, None, true))
    }

    pub(crate) fn set_transition(
        &self,
        mode: TransitionMode,
        crossfade_secs: f64,
    ) -> PlaybackSessionUpdate {
        self.apply(PlaybackIntent::SetTransition {
            mode,
            crossfade_secs,
        })
        .expect("set-transition intent is infallible")
    }

    pub(crate) fn set_playing(&self, playing: bool) -> PlaybackSessionUpdate {
        self.apply(PlaybackIntent::SetPlaying { playing })
            .expect("set-playing intent is infallible")
    }

    pub(crate) fn observe_progress(
        &self,
        position: f64,
        duration: f64,
        finished: bool,
    ) -> PlaybackSessionUpdate {
        self.apply(PlaybackIntent::ObserveProgress {
            position,
            duration,
            finished,
        })
        .expect("progress intent is infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, path: &str) -> QueueEntry {
        QueueEntry {
            id: id.to_string(),
            path: path.to_string(),
            duration_hint: 100.0,
            track_gain_db: None,
            track_peak: None,
        }
    }

    fn play(session: &PlaybackSession, entries: Vec<QueueEntry>, id: &str) {
        session
            .apply(PlaybackIntent::PlayQueue {
                entries,
                start_entry_id: Some(id.to_string()),
                autoplay: true,
                start_at: None,
                shuffle: None,
                repeat: None,
                autoplay_mode: None,
                sleep: None,
            })
            .unwrap();
    }

    #[test]
    fn duplicate_paths_keep_distinct_persistent_ids() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("first", "same.flac"), entry("second", "same.flac")],
            "first",
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.current_entry_id.as_deref(), Some("first"));
        assert_eq!(snapshot.next_entry_id.as_deref(), Some("second"));

        let update = session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        assert_eq!(update.snapshot.current_entry_id.as_deref(), Some("second"));
        match update.effect {
            Some(PlaybackEffect::Load { entry, .. }) => assert_eq!(entry.id, "second"),
            other => panic!("unexpected effect: {other:?}"),
        }
    }

    #[test]
    fn durable_restore_preserves_queue_policy_but_always_resumes_paused() {
        let source = PlaybackSession::default();
        play(
            &source,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "b",
        );
        source
            .apply(PlaybackIntent::SetModes {
                shuffle: true,
                repeat: RepeatMode::All,
                autoplay: true,
            })
            .expect("set modes");
        let persisted = source.snapshot();
        assert!(persisted.playing);

        let restored = PlaybackSession::default();
        restored.restore(persisted).expect("restore session");
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.queue.len(), 2);
        assert_eq!(snapshot.current_entry_id.as_deref(), Some("b"));
        assert!(snapshot.shuffle);
        assert_eq!(snapshot.repeat, RepeatMode::All);
        assert!(snapshot.autoplay);
        assert!(!snapshot.playing);
        assert!(snapshot.prepared.is_none());
    }

    #[test]
    fn queue_edit_invalidates_prepared_generation() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        session.set_transition(TransitionMode::Gapless, 6.0);
        let token = session.begin_prepare("b.flac", Some("b")).unwrap();
        assert!(session.is_prepare_current(&token));

        let update = session
            .apply(PlaybackIntent::MoveQueueItem {
                entry_id: "b".to_string(),
                to_index: 0,
            })
            .unwrap();
        assert!(update.prepared_invalidated);
        assert!(!session.is_prepare_current(&token));
    }

    #[test]
    fn transition_off_invalidates_decoder_and_planning_stays_available() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        session.set_transition(TransitionMode::Crossfade, 4.0);
        let token = session.begin_prepare("b.flac", Some("b")).unwrap();
        let update = session.set_transition(TransitionMode::Off, 4.0);
        assert!(update.prepared_invalidated);
        assert!(!session.is_prepare_current(&token));
        assert!(session.begin_prepare("b.flac", Some("b")).is_none());
    }

    #[test]
    fn natural_advance_honours_both_sleep_boundaries() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        session
            .apply(PlaybackIntent::SetSleep {
                sleep: SleepMode::EndTrack,
            })
            .unwrap();
        let update = session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        assert!(matches!(
            update.effect,
            Some(PlaybackEffect::Stop {
                reason: StopReason::SleepEndTrack
            })
        ));

        play(&session, vec![entry("a", "a.flac")], "a");
        session
            .apply(PlaybackIntent::SetSleep {
                sleep: SleepMode::EndQueue,
            })
            .unwrap();
        let update = session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        assert!(matches!(
            update.effect,
            Some(PlaybackEffect::Stop {
                reason: StopReason::SleepEndQueue
            })
        ));
    }

    #[test]
    fn repeat_and_history_are_native_policy() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        session
            .apply(PlaybackIntent::SetModes {
                shuffle: false,
                repeat: RepeatMode::All,
                autoplay: false,
            })
            .unwrap();
        session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        let wrapped = session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        assert_eq!(wrapped.snapshot.current_entry_id.as_deref(), Some("a"));
        assert!(wrapped.snapshot.history.iter().any(|id| id == "b"));
    }

    #[test]
    fn accounting_counts_play_then_does_not_count_skip() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        let counted = session.observe_progress(50.0, 100.0, false);
        assert!(counted.events.iter().any(|event| matches!(
            event,
            PlaybackSessionEvent::Accounting {
                kind: AccountingKind::PlayCounted,
                ..
            }
        )));
        let next = session
            .apply(PlaybackIntent::Next {
                user_triggered: true,
            })
            .unwrap();
        assert!(!next.events.iter().any(|event| matches!(
            event,
            PlaybackSessionEvent::Accounting {
                kind: AccountingKind::SkipCounted,
                ..
            }
        )));
    }

    #[test]
    fn select_entry_returns_the_only_authorized_load_effect() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![entry("a", "a.flac"), entry("b", "b.flac")],
            "a",
        );
        let update = session
            .apply(PlaybackIntent::SelectEntry {
                entry_id: "b".to_string(),
                autoplay: false,
                start_at: Some(12.5),
            })
            .unwrap();

        assert_eq!(update.snapshot.current_entry_id.as_deref(), Some("b"));
        assert!(!update.snapshot.playing);
        assert!(matches!(
            update.effect,
            Some(PlaybackEffect::Load {
                entry: QueueEntry { ref id, .. },
                autoplay: false,
                start_at: Some(position),
            }) if id == "b" && (position - 12.5).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn enqueue_and_clear_upcoming_preserve_current_identity() {
        let session = PlaybackSession::default();
        play(&session, vec![entry("a", "a.flac")], "a");
        let queued = session
            .apply(PlaybackIntent::Enqueue {
                entries: vec![entry("b", "b.flac"), entry("c", "c.flac")],
                after_current: true,
            })
            .unwrap();
        assert_eq!(queued.snapshot.next_entry_id.as_deref(), Some("b"));
        assert_eq!(queued.snapshot.queue.len(), 3);

        let cleared = session.apply(PlaybackIntent::ClearUpcoming).unwrap();
        assert_eq!(cleared.snapshot.current_entry_id.as_deref(), Some("a"));
        assert_eq!(cleared.snapshot.queue, vec![entry("a", "a.flac")]);
        assert!(cleared.snapshot.next_entry_id.is_none());
    }

    #[test]
    fn prepare_rejects_any_entry_except_the_native_plan() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![
                entry("a", "a.flac"),
                entry("b", "b.flac"),
                entry("c", "c.flac"),
            ],
            "a",
        );
        session.set_transition(TransitionMode::Gapless, 6.0);

        assert!(session.begin_prepare("c.flac", Some("c")).is_none());
        assert!(session.begin_prepare("b.flac", Some("b")).is_some());
    }

    // End-to-end policy loop: an entire user journey expressed purely as
    // session intents — load, pause/resume, scrub, natural advance, previous,
    // queue edits, teardown — asserting snapshot invariants and audio effects
    // at each step. This is the playback-loop smoke test: if any of these
    // invariants break, every transport control above the session breaks too.
    #[test]
    fn smoke_full_playback_loop_through_session_intents() {
        let session = PlaybackSession::default();
        play(
            &session,
            vec![
                entry("a", "a.flac"),
                entry("b", "b.flac"),
                entry("c", "c.flac"),
            ],
            "a",
        );
        let base_revision = session.snapshot().revision;

        // Pause → resume round-trips without changing the current track.
        let paused = session.set_playing(false);
        assert!(!paused.snapshot.playing);
        assert!(matches!(
            paused.effect,
            Some(PlaybackEffect::SetPlaying { playing: false })
        ));
        let resumed = session.set_playing(true);
        assert!(resumed.snapshot.playing);
        assert_eq!(resumed.snapshot.current_entry_id.as_deref(), Some("a"));

        // Progress observation inside the track never loads anything.
        let mid = session
            .apply(PlaybackIntent::ObserveProgress {
                position: 30.0,
                duration: 100.0,
                finished: false,
            })
            .unwrap();
        assert_eq!(mid.snapshot.current_entry_id.as_deref(), Some("a"));
        assert!(mid.effect.is_none());

        // Natural end-of-track (audio engine reports finish → non-user Next)
        // advances to "b" with a Load effect.
        let finished = session
            .apply(PlaybackIntent::Next {
                user_triggered: false,
            })
            .unwrap();
        assert_eq!(finished.snapshot.current_entry_id.as_deref(), Some("b"));
        match &finished.effect {
            Some(PlaybackEffect::Load { entry, .. }) => assert_eq!(entry.id, "b"),
            other => panic!("expected load effect after finish, got {other:?}"),
        }

        // Previous near the track start steps back to "a" and reloads it.
        let back = session
            .apply(PlaybackIntent::Previous { position: 0.5 })
            .unwrap();
        assert_eq!(back.snapshot.current_entry_id.as_deref(), Some("a"));
        assert!(matches!(back.effect, Some(PlaybackEffect::Load { .. })));

        // Queue edits keep the current entry stable.
        let moved = session
            .apply(PlaybackIntent::MoveQueueItem {
                entry_id: "c".into(),
                to_index: 1,
            })
            .unwrap();
        assert_eq!(moved.snapshot.queue[1].id, "c");
        assert_eq!(moved.snapshot.current_entry_id.as_deref(), Some("a"));

        let removed = session
            .apply(PlaybackIntent::RemoveQueueItem {
                entry_id: "b".into(),
            })
            .unwrap();
        assert_eq!(removed.snapshot.queue.len(), 2);
        assert!(!removed.snapshot.queue.iter().any(|e| e.id == "b"));

        // Tear down: Clear empties everything and stops playback with a reason.
        let cleared = session.apply(PlaybackIntent::Clear).unwrap();
        assert!(cleared.snapshot.queue.is_empty());
        assert!(cleared.snapshot.current_entry_id.is_none());
        assert!(matches!(
            cleared.effect,
            Some(PlaybackEffect::Stop { .. })
        ));

        // Every mutation advanced the revision monotonically.
        assert!(session.snapshot().revision > base_revision);
    }
}
