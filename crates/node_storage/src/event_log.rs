//! Append-only event log with hash chain, segment rotation, and replay.
//!
//! Layout:
//! - `data/events/active.log`   — current writable log
//! - `data/events/segments/`    — sealed rotated segments
//! - `data/events/index/`       — lightweight index files

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use prost::Message;
use sha2::{Digest, Sha256};
use thiserror::Error;

use node_proto::common::{HashRef, Timestamp};
use node_proto::events::EventEnvelope;

const DEFAULT_SEGMENT_THRESHOLD: u64 = 4 * 1024 * 1024; // 4 MB

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("hash chain broken at event {event_id}: expected {expected}, got {actual}")]
    ChainBroken {
        event_id: String,
        expected: String,
        actual: String,
    },
    #[error("log is empty, cannot determine head hash")]
    EmptyLog,
}

pub type Result<T> = std::result::Result<T, EventLogError>;

pub struct EventLog {
    #[allow(dead_code)]
    events_dir: PathBuf,
    active_path: PathBuf,
    segments_dir: PathBuf,
    segment_threshold: u64,
    head_hash: String,
    event_count: u64,
}

impl EventLog {
    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with_threshold(data_dir, DEFAULT_SEGMENT_THRESHOLD)
    }

    pub fn open_with_threshold(data_dir: &Path, segment_threshold: u64) -> Result<Self> {
        let events_dir = data_dir.join("events");
        let active_path = events_dir.join("active.log");
        let segments_dir = events_dir.join("segments");
        let index_dir = events_dir.join("index");

        fs::create_dir_all(&segments_dir)?;
        fs::create_dir_all(&index_dir)?;

        if !active_path.exists() {
            File::create(&active_path)?;
        }

        let mut log = Self {
            events_dir,
            active_path,
            segments_dir,
            segment_threshold,
            head_hash: String::new(),
            event_count: 0,
        };

        log.rebuild_head()?;
        Ok(log)
    }

    fn rebuild_head(&mut self) -> Result<()> {
        let mut last_hash = String::new();
        let mut count = 0u64;

        for segment_path in self.segment_paths()? {
            let events = Self::read_log_file(&segment_path)?;
            if let Some(last) = events.last() {
                last_hash = last
                    .event_hash
                    .as_ref()
                    .map(|h| h.sha256.clone())
                    .unwrap_or_default();
            }
            count += events.len() as u64;
        }

        let active_events = Self::read_log_file(&self.active_path)?;
        if let Some(last) = active_events.last() {
            last_hash = last
                .event_hash
                .as_ref()
                .map(|h| h.sha256.clone())
                .unwrap_or_default();
        }
        count += active_events.len() as u64;

        self.head_hash = last_hash;
        self.event_count = count;
        Ok(())
    }

    fn segment_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths: Vec<PathBuf> = Vec::new();
        if self.segments_dir.exists() {
            for entry in fs::read_dir(&self.segments_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("log") {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        Ok(paths)
    }

    fn read_log_file(path: &Path) -> Result<Vec<EventEnvelope>> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut events = Vec::new();
        let mut len_buf = [0u8; 4];

        loop {
            match reader.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut msg_buf = vec![0u8; len];
            reader.read_exact(&mut msg_buf)?;
            let event = EventEnvelope::decode(msg_buf.as_slice())?;
            events.push(event);
        }

        Ok(events)
    }

    /// Compute the hash of an event envelope (deterministic from serialized form).
    fn compute_event_hash(event: &EventEnvelope) -> String {
        let encoded = event.encode_to_vec();
        hex::encode(Sha256::digest(&encoded))
    }

    /// Append an event. Sets `prev_hash`, `event_hash`, and `ts` if missing.
    pub fn append(&mut self, mut event: EventEnvelope) -> Result<EventEnvelope> {
        event.prev_hash = if self.head_hash.is_empty() {
            None
        } else {
            Some(HashRef {
                sha256: self.head_hash.clone(),
            })
        };

        if event.ts.is_none() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            event.ts = Some(Timestamp { unix_ms: now });
        }

        // Compute hash before setting it (hash covers prev_hash but not event_hash itself)
        let mut for_hash = event.clone();
        for_hash.event_hash = None;
        let hash = Self::compute_event_hash(&for_hash);
        event.event_hash = Some(HashRef {
            sha256: hash.clone(),
        });

        let encoded = event.encode_to_vec();
        let len = encoded.len() as u32;

        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.active_path)?;
        f.write_all(&len.to_le_bytes())?;
        f.write_all(&encoded)?;
        f.flush()?;

        self.head_hash = hash;
        self.event_count += 1;

        self.maybe_rotate()?;

        tracing::debug!(
            event_id = %event.event_id,
            hash = %self.head_hash,
            count = self.event_count,
            "event appended"
        );

        Ok(event)
    }

    fn maybe_rotate(&mut self) -> Result<()> {
        let meta = fs::metadata(&self.active_path)?;
        if meta.len() >= self.segment_threshold {
            self.rotate()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        let segment_name = format!("segment_{:010}.log", self.event_count);
        let dest = self.segments_dir.join(segment_name);
        fs::rename(&self.active_path, &dest)?;
        File::create(&self.active_path)?;
        tracing::info!(segment = %dest.display(), "log segment rotated");
        Ok(())
    }

    /// Replay all events in order (segments first, then active log).
    pub fn replay(&self) -> Result<Vec<EventEnvelope>> {
        let mut all = Vec::new();
        for seg in self.segment_paths()? {
            all.extend(Self::read_log_file(&seg)?);
        }
        all.extend(Self::read_log_file(&self.active_path)?);
        Ok(all)
    }

    /// Verify the hash chain of all stored events.
    pub fn verify_chain(&self) -> Result<()> {
        let events = self.replay()?;
        let mut expected_prev = String::new();

        for event in &events {
            let actual_prev = event
                .prev_hash
                .as_ref()
                .map(|h| h.sha256.as_str())
                .unwrap_or("");

            if actual_prev != expected_prev {
                return Err(EventLogError::ChainBroken {
                    event_id: event.event_id.clone(),
                    expected: expected_prev,
                    actual: actual_prev.to_string(),
                });
            }

            let mut for_hash = event.clone();
            for_hash.event_hash = None;
            let computed = Self::compute_event_hash(&for_hash);

            let stored = event
                .event_hash
                .as_ref()
                .map(|h| h.sha256.as_str())
                .unwrap_or("");

            if computed != stored {
                return Err(EventLogError::ChainBroken {
                    event_id: event.event_id.clone(),
                    expected: computed,
                    actual: stored.to_string(),
                });
            }

            expected_prev = computed;
        }

        Ok(())
    }

    pub fn head_hash(&self) -> &str {
        &self.head_hash
    }

    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// Tail: return events starting from a given sequence number (0-indexed).
    pub fn tail(&self, from: u64) -> Result<Vec<EventEnvelope>> {
        let all = self.replay()?;
        Ok(all.into_iter().skip(from as usize).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;
    use node_proto::events::*;
    use tempfile::TempDir;

    fn make_case_event(id: &str, title: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: id.to_string(),
            r#type: EventType::CaseCreated as i32,
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                case_id: format!("case-{id}"),
                title: title.to_string(),
                summary: format!("Summary for {title}"),
                content_ref: Some(HashRef {
                    sha256: "placeholder".into(),
                }),
                shareable: false,
            })),
            ..Default::default()
        }
    }

    #[test]
    fn append_and_replay() {
        let tmp = TempDir::new().unwrap();
        let mut log = EventLog::open(tmp.path()).unwrap();

        for i in 0..10 {
            let evt = make_case_event(&format!("evt-{i}"), &format!("Case {i}"));
            log.append(evt).unwrap();
        }

        assert_eq!(log.event_count(), 10);

        let events = log.replay().unwrap();
        assert_eq!(events.len(), 10);
        for (i, evt) in events.iter().enumerate() {
            assert_eq!(evt.event_id, format!("evt-{i}"));
            assert!(evt.event_hash.is_some());
            assert!(evt.ts.is_some());
        }
    }

    #[test]
    fn hash_chain_valid() {
        let tmp = TempDir::new().unwrap();
        let mut log = EventLog::open(tmp.path()).unwrap();

        for i in 0..20 {
            log.append(make_case_event(&format!("e{i}"), &format!("C{i}")))
                .unwrap();
        }

        log.verify_chain().unwrap();

        let events = log.replay().unwrap();
        assert!(events[0].prev_hash.is_none());
        for i in 1..events.len() {
            assert_eq!(
                events[i]
                    .prev_hash
                    .as_ref()
                    .map(|h| h.sha256.as_str())
                    .unwrap_or(""),
                events[i - 1]
                    .event_hash
                    .as_ref()
                    .map(|h| h.sha256.as_str())
                    .unwrap_or("")
            );
        }
    }

    #[test]
    fn segment_rotation() {
        let tmp = TempDir::new().unwrap();
        let mut log = EventLog::open_with_threshold(tmp.path(), 512).unwrap();

        for i in 0..100 {
            log.append(make_case_event(
                &format!("e{i}"),
                &format!("Case number {i} with some extra text to inflate size"),
            ))
            .unwrap();
        }

        let segments: Vec<_> = fs::read_dir(tmp.path().join("events").join("segments"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !segments.is_empty(),
            "at least one segment should have been created"
        );

        let events = log.replay().unwrap();
        assert_eq!(events.len(), 100);
        log.verify_chain().unwrap();
    }

    #[test]
    fn reopen_preserves_state() {
        let tmp = TempDir::new().unwrap();
        let head_hash;
        {
            let mut log = EventLog::open(tmp.path()).unwrap();
            for i in 0..5 {
                log.append(make_case_event(&format!("e{i}"), &format!("C{i}")))
                    .unwrap();
            }
            head_hash = log.head_hash().to_string();
        }

        let log2 = EventLog::open(tmp.path()).unwrap();
        assert_eq!(log2.event_count(), 5);
        assert_eq!(log2.head_hash(), head_hash);
        log2.verify_chain().unwrap();
    }

    #[test]
    fn tail_from_offset() {
        let tmp = TempDir::new().unwrap();
        let mut log = EventLog::open(tmp.path()).unwrap();

        for i in 0..10 {
            log.append(make_case_event(&format!("e{i}"), &format!("C{i}")))
                .unwrap();
        }

        let tail = log.tail(7).unwrap();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].event_id, "e7");
        assert_eq!(tail[1].event_id, "e8");
        assert_eq!(tail[2].event_id, "e9");
    }

    #[test]
    fn empty_log() {
        let tmp = TempDir::new().unwrap();
        let log = EventLog::open(tmp.path()).unwrap();
        assert_eq!(log.event_count(), 0);
        assert!(log.head_hash().is_empty());
        let events = log.replay().unwrap();
        assert!(events.is_empty());
        log.verify_chain().unwrap();
    }

    #[test]
    fn append_10k_events() {
        let tmp = TempDir::new().unwrap();
        let mut log = EventLog::open_with_threshold(tmp.path(), 64 * 1024).unwrap();

        for i in 0..10_000 {
            log.append(make_case_event(&format!("e{i}"), &format!("C{i}")))
                .unwrap();
        }

        assert_eq!(log.event_count(), 10_000);
        let events = log.replay().unwrap();
        assert_eq!(events.len(), 10_000);
        log.verify_chain().unwrap();
    }
}
