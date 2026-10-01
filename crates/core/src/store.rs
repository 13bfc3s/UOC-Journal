//! Append-only in-memory journal owned by the UI thread.

use rustc_hash::FxHashMap;

use crate::model::{Channel, Entry};
use crate::names::Person;
use crate::pipeline::{Batch, Session};

pub struct Speaker {
    pub name: Box<str>,
    pub lower: Box<str>,
}

#[derive(Default)]
pub struct Store {
    entries: Vec<Entry>,
    text: String,
    lower: String,
    speakers: Vec<Speaker>,
    sessions: Vec<Session>,
    session_chars_lower: Vec<String>,
    people: FxHashMap<String, Person>,
    /// Incremented whenever ids are invalidated (reset). Views compare it.
    epoch: u64,
    /// Lowest reclassified id of each reclassification event, in order.
    reclass_log: Vec<u32>,
    /// Bumped on any change, for cheap "did anything happen" checks.
    revision: u64,
    people_revision: u64,
    counts: [usize; Channel::COUNT],
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn people_revision(&self) -> u64 {
        self.people_revision
    }

    pub fn reclass_log(&self) -> &[u32] {
        &self.reclass_log
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[inline]
    pub fn entry(&self, id: u32) -> &Entry {
        &self.entries[id as usize]
    }

    #[inline]
    pub fn text(&self, e: &Entry) -> &str {
        &self.text[e.text_off as usize..(e.text_off + e.text_len) as usize]
    }

    /// Lower-cased text, byte-aligned with [`Store::text`].
    #[inline]
    pub fn lower(&self, e: &Entry) -> &str {
        &self.lower[e.text_off as usize..(e.text_off + e.text_len) as usize]
    }

    #[inline]
    pub fn speaker(&self, e: &Entry) -> &str {
        self.speakers
            .get(e.speaker as usize)
            .map(|s| &*s.name)
            .unwrap_or("")
    }

    #[inline]
    pub fn speaker_lower(&self, e: &Entry) -> &str {
        self.speakers
            .get(e.speaker as usize)
            .map(|s| &*s.lower)
            .unwrap_or("")
    }

    pub fn speaker_by_id(&self, id: u32) -> &str {
        self.speakers
            .get(id as usize)
            .map(|s| &*s.name)
            .unwrap_or("")
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn session(&self, id: u16) -> Option<&Session> {
        self.sessions.get(id as usize)
    }

    /// Character name for an entry, if its session knows one.
    pub fn character(&self, e: &Entry) -> Option<&str> {
        self.sessions
            .get(e.session as usize)
            .and_then(|s| s.character.as_deref())
    }

    /// Lower-case character name per session id ("" when unknown).
    pub fn session_chars_lower(&self) -> &[String] {
        &self.session_chars_lower
    }

    /// Distinct character names seen, in first-seen order.
    pub fn characters(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.sessions {
            if let Some(c) = &s.character {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        out
    }

    pub fn people(&self) -> &FxHashMap<String, Person> {
        &self.people
    }

    pub fn person(&self, name: &str) -> Option<&Person> {
        self.people.get(name)
    }

    /// Newest timestamp in the store.
    pub fn last_time(&self) -> Option<u32> {
        self.entries.last().map(|e| e.time)
    }

    pub fn clear(&mut self) {
        let epoch = self.epoch + 1;
        let revision = self.revision + 1;
        *self = Store::default();
        self.epoch = epoch;
        self.revision = revision;
        self.people_revision = revision;
    }

    /// Append a batch from the pipeline. Returns false if the batch did not line up
    /// with the store (the caller should then request a full reload).
    pub fn apply(&mut self, mut batch: Batch) -> bool {
        if batch.reset {
            self.clear();
        }
        self.revision += 1;
        let base = self.text.len();
        if base + batch.text.len() > u32::MAX as usize {
            // 4 GiB of journal text: refuse rather than corrupt offsets.
            return false;
        }
        if !batch.entries.is_empty() && batch.first_id as usize != self.entries.len() {
            return false;
        }
        for name in batch.new_speakers.drain(..) {
            let mut lower = String::with_capacity(name.len());
            crate::pipeline::push_lower(&mut lower, &name);
            self.speakers.push(Speaker {
                name: name.into_boxed_str(),
                lower: lower.into_boxed_str(),
            });
        }
        for (id, s) in batch.sessions.drain(..) {
            let id = id as usize;
            if id >= self.sessions.len() {
                self.sessions.resize(id + 1, Session::default());
                self.session_chars_lower.resize(id + 1, String::new());
            }
            self.session_chars_lower[id] = s
                .character
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            self.sessions[id] = s;
        }
        if !batch.people.is_empty() {
            self.people_revision += 1;
            for p in batch.people.drain(..) {
                self.people.insert(p.name.clone(), p);
            }
        }
        self.text.push_str(&batch.text);
        self.lower.push_str(&batch.lower);
        let base = base as u32;
        self.entries.reserve(batch.entries.len());
        for mut e in batch.entries {
            e.text_off += base;
            self.counts[e.channel.index()] += 1;
            self.entries.push(e);
        }
        if !batch.reclass.is_empty() {
            let mut min = u32::MAX;
            for (id, ch) in batch.reclass {
                if let Some(e) = self.entries.get_mut(id as usize) {
                    self.counts[e.channel.index()] -= 1;
                    self.counts[ch.index()] += 1;
                    e.channel = ch;
                    min = min.min(id);
                }
            }
            if min != u32::MAX {
                self.reclass_log.push(min);
            }
        }
        true
    }

    /// Count of entries per channel (for the status bar / chips).
    pub fn channel_counts(&self) -> [usize; Channel::COUNT] {
        self.counts
    }

    /// Approximate heap usage in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.text.capacity()
            + self.lower.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Pipeline;
    use std::path::PathBuf;

    #[test]
    fn apply_batches_and_reclass() {
        let mut p = Pipeline::default();
        let mut fc = p.open_file(0, PathBuf::from("2026_03_14_18_02_11_journal.txt"));
        let mut store = Store::new();
        p.process_chunk(&mut fc, "[03/14/2026 18:02]  System: Welcome Aldric Thorne!\n[03/14/2026 18:03]  Bramblewick: Master Carpenter");
        assert!(store.apply(p.take_batch()));
        assert_eq!(store.len(), 2);
        assert_eq!(store.text(store.entry(1)), "Master Carpenter");
        assert_eq!(store.entry(1).channel, Channel::Speech);
        assert_eq!(store.character(store.entry(0)), Some("Aldric Thorne"));
        p.process_chunk(&mut fc, "[03/14/2026 18:03]  Bramblewick: Bramblewick");
        assert!(store.apply(p.take_batch()));
        assert_eq!(store.entry(1).channel, Channel::Names);
        assert_eq!(store.reclass_log(), &[1]);
        assert_eq!(store.speaker(store.entry(2)), "Bramblewick");
        assert_eq!(store.lower(store.entry(2)), "bramblewick");
        assert!(store.person("Bramblewick").is_some());
        let mut manual = [0usize; Channel::COUNT];
        for e in store.entries() {
            manual[e.channel.index()] += 1;
        }
        assert_eq!(store.channel_counts(), manual);
    }
}
