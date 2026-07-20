use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use rodio::{Decoder, Source};

/// Encoded audio kept in memory per decoder. Current, prepared-next, and a
/// crossfading predecessor therefore have a hard aggregate upper bound instead
/// of each retaining an entire file.
const READ_AHEAD_CAPACITY: usize = 2 * 1024 * 1024;
const READ_AHEAD_PREFILL: usize = 512 * 1024;
const READ_CHUNK: usize = 64 * 1024;

#[derive(Debug)]
struct ReaderState {
    buffer: VecDeque<u8>,
    eof: bool,
    error: Option<(io::ErrorKind, String)>,
    shutdown: bool,
    epoch: u64,
    requested_seek: Option<u64>,
    peak_buffered: usize,
    underruns: u64,
}

impl ReaderState {
    fn new() -> Self {
        Self {
            buffer: VecDeque::with_capacity(READ_AHEAD_CAPACITY),
            eof: false,
            error: None,
            shutdown: false,
            epoch: 0,
            requested_seek: None,
            peak_buffered: 0,
            underruns: 0,
        }
    }
}

#[derive(Debug)]
struct ReaderShared {
    state: Mutex<ReaderState>,
    data_ready: Condvar,
    space_ready: Condvar,
}

/// Seekable, bounded producer/consumer reader.
///
/// The worker performs filesystem reads away from Rodio's mixer thread. The
/// decoder consumes encoded bytes from a bounded ring and blocks only if the
/// configured read-ahead is genuinely exhausted. Seeking invalidates an
/// in-flight read by epoch so stale bytes can never be appended after a seek.
#[derive(Debug)]
pub(crate) struct ReadAheadFile {
    shared: Arc<ReaderShared>,
    position: u64,
    len: u64,
}

impl ReadAheadFile {
    fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let shared = Arc::new(ReaderShared {
            state: Mutex::new(ReaderState::new()),
            data_ready: Condvar::new(),
            space_ready: Condvar::new(),
        });
        let worker_shared = shared.clone();
        thread::Builder::new()
            .name("audio-read-ahead".to_string())
            .spawn(move || read_ahead_worker(file, worker_shared))?;

        // Prime enough encoded data to absorb normal filesystem scheduling
        // jitter before handing the decoder to the audio pipeline.
        let target = READ_AHEAD_PREFILL.min(len as usize);
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.buffer.len() < target && !state.eof && state.error.is_none() {
            state = shared
                .data_ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if let Some((kind, message)) = state.error.take() {
            state.shutdown = true;
            shared.space_ready.notify_all();
            return Err(io::Error::new(kind, message));
        }
        drop(state);

        Ok(Self {
            shared,
            position: 0,
            len,
        })
    }

    #[cfg(test)]
    fn stats(&self) -> (usize, usize, u64) {
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.buffer.len(), state.peak_buffered, state.underruns)
    }
}

impl Read for ReadAheadFile {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }

        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut waited = false;
        while state.buffer.is_empty() && !state.eof && state.error.is_none() {
            waited = true;
            state = self
                .shared
                .data_ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if waited {
            state.underruns = state.underruns.saturating_add(1);
        }

        let count = output.len().min(state.buffer.len());
        for slot in &mut output[..count] {
            *slot = state.buffer.pop_front().expect("buffer length was checked");
        }
        if count > 0 {
            self.position = self.position.saturating_add(count as u64);
            self.shared.space_ready.notify_one();
            return Ok(count);
        }
        if let Some((kind, message)) = state.error.take() {
            return Err(io::Error::new(kind, message));
        }
        Ok(0)
    }
}

impl Seek for ReadAheadFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::Current(offset) => self.position as i128 + offset as i128,
            SeekFrom::End(offset) => self.len as i128 + offset as i128,
        };
        if !(0..=self.len as i128).contains(&target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek target is outside the audio file",
            ));
        }
        let target = target as u64;
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = state.epoch.wrapping_add(1);
        state.requested_seek = Some(target);
        state.buffer.clear();
        state.eof = false;
        state.error = None;
        self.position = target;
        self.shared.space_ready.notify_all();
        Ok(target)
    }
}

impl Drop for ReadAheadFile {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        self.shared.space_ready.notify_all();
        self.shared.data_ready.notify_all();
    }
}

fn read_ahead_worker(mut file: File, shared: Arc<ReaderShared>) {
    let mut chunk = vec![0_u8; READ_CHUNK];
    loop {
        let (epoch, read_len) = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while !state.shutdown
                && state.requested_seek.is_none()
                && (state.eof || state.buffer.len() >= READ_AHEAD_CAPACITY)
            {
                state = shared
                    .space_ready
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            if state.shutdown {
                return;
            }
            if let Some(target) = state.requested_seek.take() {
                if let Err(error) = file.seek(SeekFrom::Start(target)) {
                    state.error = Some((error.kind(), error.to_string()));
                    shared.data_ready.notify_all();
                    continue;
                }
            }
            let available = READ_AHEAD_CAPACITY.saturating_sub(state.buffer.len());
            (state.epoch, available.min(chunk.len()))
        };

        let result = file.read(&mut chunk[..read_len]);
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.epoch != epoch {
            // A seek raced this read. The new epoch's worker iteration will
            // reposition before publishing any bytes.
            continue;
        }
        match result {
            Ok(0) => state.eof = true,
            Ok(count) => {
                state.buffer.extend(&chunk[..count]);
                state.peak_buffered = state.peak_buffered.max(state.buffer.len());
            }
            Err(error) => state.error = Some((error.kind(), error.to_string())),
        }
        shared.data_ready.notify_all();
    }
}

pub(crate) type AudioDecoder = Decoder<ReadAheadFile>;

pub(crate) fn build_decoder(path: &Path) -> Result<(AudioDecoder, f64), String> {
    let byte_len = path.metadata().map_err(|e| e.to_string())?.len();
    let reader = ReadAheadFile::open(path).map_err(|e| e.to_string())?;
    let decoder = Decoder::builder()
        .with_data(reader)
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
        .map_err(|e| e.to_string())?;
    let duration = decoder
        .total_duration()
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Ok((decoder, duration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_file(bytes: &[u8]) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ts-music-read-ahead-{nonce}.bin"));
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn read_ahead_is_bounded_and_preserves_bytes() {
        let input: Vec<u8> = (0..(READ_AHEAD_CAPACITY * 3 + 17))
            .map(|index| (index % 251) as u8)
            .collect();
        let path = test_file(&input);
        let mut reader = ReadAheadFile::open(&path).unwrap();
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        let (_, peak, _) = reader.stats();
        fs::remove_file(path).unwrap();

        assert_eq!(output, input);
        assert!(peak <= READ_AHEAD_CAPACITY);
    }

    #[test]
    fn seek_discards_in_flight_old_epoch_bytes() {
        let input: Vec<u8> = (0..(READ_AHEAD_CAPACITY + 4096))
            .map(|index| (index % 239) as u8)
            .collect();
        let path = test_file(&input);
        let mut reader = ReadAheadFile::open(&path).unwrap();
        let target = (READ_AHEAD_CAPACITY / 2 + 123) as u64;
        reader.seek(SeekFrom::Start(target)).unwrap();
        let mut output = vec![0; 8192];
        reader.read_exact(&mut output).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(
            output,
            input[target as usize..target as usize + output.len()]
        );
    }

    #[test]
    fn prefill_absorbs_playback_rate_consumption_without_underrun() {
        let input = vec![0x5a; READ_AHEAD_CAPACITY * 2];
        let path = test_file(&input);
        let mut reader = ReadAheadFile::open(&path).unwrap();
        let mut block = vec![0; 4096];
        for _ in 0..128 {
            reader.read_exact(&mut block).unwrap();
            thread::sleep(std::time::Duration::from_micros(100));
        }
        let (_, peak, underruns) = reader.stats();
        fs::remove_file(path).unwrap();

        assert_eq!(underruns, 0, "read-ahead starved under paced consumption");
        assert!(peak <= READ_AHEAD_CAPACITY);
    }
}
