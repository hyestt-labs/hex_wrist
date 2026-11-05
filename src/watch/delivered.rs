//! The 24 h table of recordings already delivered. It outlives the History
//! retention and restarts, so a watch that resends a recording is told
//! "duplicate", and `/status` never falls back to `awaiting-audio` and makes
//! the watch send the audio again. It stores no text. The file is the same
//! JSON array Wristper's `delivered-ids.json` holds, so an existing one loads.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::Timestamp;
use super::recording::{PasteMarks, State};

pub(crate) const RETENTION: Duration = Duration::from_secs(86_400);
pub(crate) const DEFAULT_CAPACITY: usize = 5_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) state: String,
    pub(crate) verified: bool,
    pub(crate) verification: String,
    pub(crate) truncated: bool,
    pub(crate) entered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at: Option<String>,
    pub(crate) recorded_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DeliveredIds {
    /// Oldest first by `recorded_at`. Timestamps share one fixed-width format,
    /// so string order is time order (Wristper and the Python receiver rely
    /// on the same).
    entries: Vec<Entry>,
    capacity: usize,
}

impl Default for DeliveredIds {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl DeliveredIds {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
        }
    }

    /// An unreadable or malformed file reads as empty, like Wristper's
    /// `try?` decode. One bad entry drops the whole table.
    pub(crate) fn from_json(bytes: &[u8]) -> Self {
        let mut entries: Vec<Entry> = serde_json::from_slice(bytes).unwrap_or_default();
        entries.sort_by(|a, b| a.recorded_at.cmp(&b.recorded_at));
        Self {
            entries,
            capacity: DEFAULT_CAPACITY,
        }
    }

    pub(crate) fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    pub(crate) fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(&self.entries).unwrap_or_else(|_| b"[]".to_vec())
    }

    pub(crate) fn len(&self, now: SystemTime) -> usize {
        let cutoff = cutoff(now);
        self.entries
            .iter()
            .filter(|entry| entry.recorded_at >= cutoff)
            .count()
    }

    pub(crate) fn get(&self, id: &str, now: SystemTime) -> Option<&Entry> {
        let cutoff = cutoff(now);
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.id == id && entry.recorded_at >= cutoff)
    }

    /// Notes `id`'s final `state` and the marks its paste left. A known id
    /// keeps its first `recorded_at`.
    pub(crate) fn record(
        &mut self,
        id: &str,
        state: State,
        paste: PasteMarks,
        truncated: bool,
        completed_at: Option<String>,
        now: SystemTime,
    ) {
        let cutoff = cutoff(now);
        self.entries.retain(|entry| entry.recorded_at >= cutoff);
        let verification = paste.verification.as_str().to_owned();
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.state = state.name().to_owned();
            entry.verified = paste.verified;
            entry.verification = verification;
            entry.truncated = truncated;
            entry.entered = paste.entered;
            entry.completed_at = completed_at;
        } else {
            self.entries.push(Entry {
                id: id.to_owned(),
                state: state.name().to_owned(),
                verified: paste.verified,
                verification,
                truncated,
                entered: paste.entered,
                completed_at,
                recorded_at: Timestamp::string(now),
            });
        }
        if self.entries.len() > self.capacity {
            let excess = self.entries.len() - self.capacity;
            self.entries.drain(..excess);
        }
    }
}

fn cutoff(now: SystemTime) -> String {
    Timestamp::string(now.checked_sub(RETENTION).unwrap_or(UNIX_EPOCH))
}

/// A missing file is an empty table.
pub(crate) fn load(path: &Path) -> DeliveredIds {
    std::fs::read(path)
        .map(|bytes| DeliveredIds::from_json(&bytes))
        .unwrap_or_default()
}

impl DeliveredIds {
    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        super::write_owner_only(path, &self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::recording::Verification;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    fn read_back() -> PasteMarks {
        PasteMarks::sent(Verification::ReadBack, false)
    }

    const ID_A: &str = "11111111-1111-1111-1111-111111111111";
    const ID_B: &str = "22222222-2222-2222-2222-222222222222";
    const ID_C: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn a_file_written_by_wristper_loads_and_is_found() {
        // Swift's JSONEncoder omits a nil `completed_at`, so one entry lacks it.
        let now = at(1_000);
        let recorded = Timestamp::string(at(500));
        let swift = format!(
            r#"[
              {{"id":"{ID_A}","state":"pasted","verified":true,"verification":"read-back",
                "truncated":false,"entered":true,"completed_at":"{recorded}","recorded_at":"{recorded}"}},
              {{"id":"{ID_B}","state":"saved","verified":false,"verification":"none",
                "truncated":true,"entered":false,"recorded_at":"{recorded}"}}
            ]"#
        );
        let table = DeliveredIds::from_json(swift.as_bytes());
        let a = table.get(ID_A, now).expect("entry with completed_at");
        assert!(a.entered && a.verified);
        assert_eq!(a.completed_at.as_deref(), Some(recorded.as_str()));
        let b = table.get(ID_B, now).expect("entry without completed_at");
        assert_eq!(b.completed_at, None);
        assert!(b.truncated);
    }

    #[test]
    fn unreadable_input_reads_as_an_empty_table() {
        assert_eq!(DeliveredIds::from_json(b"not json").len(at(0)), 0);
        assert_eq!(DeliveredIds::from_json(b"{}").len(at(0)), 0);
    }

    #[test]
    fn output_omits_completed_at_when_absent() {
        let mut table = DeliveredIds::default();
        table.record(
            ID_A,
            State::Saved,
            PasteMarks::default(),
            false,
            None,
            at(0),
        );
        let text = String::from_utf8(table.to_json()).unwrap();
        assert!(!text.contains("completed_at"), "{text}");
        assert!(text.starts_with('['));
    }

    #[test]
    fn an_entry_past_24_hours_is_not_returned_and_is_dropped_on_the_next_record() {
        let mut table = DeliveredIds::default();
        table.record(ID_A, State::Pasted, read_back(), false, None, at(0));
        let just_inside = at(86_400);
        assert!(table.get(ID_A, just_inside).is_some());
        let later = at(86_401);
        assert!(table.get(ID_A, later).is_none());
        assert_eq!(table.len(later), 0);
        table.record(ID_B, State::Pasted, read_back(), false, None, later);
        assert_eq!(table.entries.len(), 1);
        assert_eq!(table.entries[0].id, ID_B);
    }

    #[test]
    fn capacity_trims_the_oldest_first() {
        let mut table = DeliveredIds::new(2);
        table.record(ID_A, State::Pasted, read_back(), false, None, at(1));
        table.record(ID_B, State::Pasted, read_back(), false, None, at(2));
        table.record(ID_C, State::Pasted, read_back(), false, None, at(3));
        let now = at(10);
        assert!(table.get(ID_A, now).is_none());
        assert!(table.get(ID_B, now).is_some());
        assert!(table.get(ID_C, now).is_some());
    }

    #[test]
    fn recording_a_known_id_updates_it_in_place() {
        let mut table = DeliveredIds::default();
        table.record(
            ID_A,
            State::Unverified,
            PasteMarks::sent(Verification::None, false),
            false,
            None,
            at(1),
        );
        let first_recorded_at = table.entries[0].recorded_at.clone();
        table.record(
            ID_A,
            State::Pasted,
            read_back(),
            true,
            Some("done".into()),
            at(5),
        );
        assert_eq!(table.entries.len(), 1);
        let entry = &table.entries[0];
        assert_eq!(entry.state, "pasted");
        assert!(entry.verified && entry.truncated);
        assert_eq!(entry.verification, "read-back");
        assert_eq!(entry.completed_at.as_deref(), Some("done"));
        assert_eq!(entry.recorded_at, first_recorded_at);
    }

    #[test]
    fn the_paste_marks_are_stored_whatever_the_state() {
        let mut table = DeliveredIds::default();
        let marks = PasteMarks::sent(Verification::Terminal, true);
        table.record(ID_A, State::Undone, marks, false, None, at(0));
        let entry = table.get(ID_A, at(1)).unwrap();
        assert_eq!(
            (
                entry.state.as_str(),
                entry.verified,
                entry.verification.as_str(),
                entry.entered
            ),
            ("undone", true, "terminal", true)
        );
    }

    #[test]
    fn save_then_load_round_trips_and_is_owner_only() {
        let directory = std::env::temp_dir().join(format!(
            "hex-delivered-{}-{}",
            std::process::id(),
            Timestamp::string(SystemTime::now()).replace(':', "")
        ));
        let folder = directory.join("nested");
        let path = folder.join("delivered-ids.json");
        let mut table = DeliveredIds::default();
        table.record(
            ID_A,
            State::Pasted,
            read_back(),
            false,
            None,
            SystemTime::now(),
        );
        table.save(&path).unwrap();
        let loaded = load(&path);
        assert!(loaded.get(ID_A, SystemTime::now()).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&folder), 0o700);
            let names: Vec<_> = std::fs::read_dir(&folder)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(
                names,
                ["delivered-ids.json"],
                "no staging file is left behind"
            );
        }
        assert_eq!(
            load(&directory.join("missing.json")).len(SystemTime::now()),
            0
        );
        let _ = std::process::Command::new("/usr/bin/trash")
            .arg(&directory)
            .status();
    }
}
