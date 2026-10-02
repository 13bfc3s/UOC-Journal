//! Turns raw lines from one or more journal files into classified entries.
//!
//! The pipeline owns all state that spans lines: which character is logged in on
//! each file, the name registry, the speaker interner and the de-duplicator. Its
//! output is a [`Batch`] that the UI thread appends to its [`crate::Store`].

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use rustc_hash::{FxHashMap, FxHasher};

use crate::classify::{looks_like_title, Classifier, CompiledRule, Ctx, Label};
use crate::model::{flags, Channel, Entry};
use crate::names::{NameBook, Person, PersonKind};
use crate::parse::{self, StampParser};

/// One logged-in character on one journal file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub file: u16,
    pub file_name: String,
    pub path: PathBuf,
    pub character: Option<String>,
}

/// Output of the pipeline, applied to the UI store in order.
#[derive(Default, Debug)]
pub struct Batch {
    /// Start over: drop everything the store holds.
    pub reset: bool,
    /// Global id of `entries[0]`. Must equal the store length when applied.
    pub first_id: u32,
    /// Message text arena; entry offsets are relative to it.
    pub text: String,
    /// Lower-cased copy of `text` with identical byte layout.
    pub lower: String,
    pub entries: Vec<Entry>,
    /// Speakers first seen in this batch; ids continue the store's numbering.
    pub new_speakers: Vec<String>,
    /// Sessions created or changed (by id).
    pub sessions: Vec<(u16, Session)>,
    pub people: Vec<Person>,
    /// Entries from earlier batches whose channel changed.
    pub reclass: Vec<(u32, Channel)>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        !self.reset
            && self.entries.is_empty()
            && self.sessions.is_empty()
            && self.people.is_empty()
            && self.reclass.is_empty()
            && self.new_speakers.is_empty()
    }
}

/// Lower-case `s` into `out` without changing byte lengths (characters whose
/// lower-case form has a different UTF-8 length are kept as-is), so offsets in the
/// lower-case arena line up with the original.
pub fn push_lower(out: &mut String, s: &str) {
    if s.is_ascii() {
        out.extend(s.bytes().map(|b| b.to_ascii_lowercase() as char));
        return;
    }
    for ch in s.chars() {
        let mut lower = ch.to_lowercase();
        match (lower.next(), lower.next()) {
            (Some(l), None) if l.len_utf8() == ch.len_utf8() => out.push(l),
            _ => out.push(ch),
        }
    }
}

#[derive(Clone, Copy)]
struct Recent {
    id: u32,
    speaker: u32,
    time: u32,
    channel: Channel,
    title_like: bool,
}

/// Per-file parsing state.
pub struct FileCursor {
    pub file: u16,
    pub name: String,
    pub path: PathBuf,
    stamps: StampParser,
    session: u16,
    self_name: Option<String>,
    staff_body: bool,
    recent: VecDeque<Recent>,
    last: Option<(u32, Channel, u32, u8)>,
}

impl FileCursor {
    pub fn self_name(&self) -> Option<&str> {
        self.self_name.as_deref()
    }

    pub fn session(&self) -> u16 {
        self.session
    }

    pub fn forget_recent(&mut self) {
        self.recent.clear();
    }
}

/// Tracks recently seen chat so that the same message received by several
/// running clients is shown once.
#[derive(Default)]
struct Deduper {
    seen: FxHashMap<u64, (u32, u16)>,
    newest: u32,
}

impl Deduper {
    fn check(&mut self, key: u64, time: u32, file: u16) -> bool {
        if time > self.newest {
            self.newest = time;
            if self.seen.len() > 4096 {
                let cutoff = time.saturating_sub(2);
                self.seen.retain(|_, (t, _)| *t >= cutoff);
            }
        }
        match self.seen.get(&key) {
            Some(&(t, f)) if f != file && time.abs_diff(t) <= 1 => true,
            _ => {
                self.seen.insert(key, (time, file));
                false
            }
        }
    }

    fn clear(&mut self) {
        self.seen.clear();
        self.newest = 0;
    }
}

pub struct Pipeline {
    classifier: Classifier,
    rules: Vec<CompiledRule>,
    speakers: FxHashMap<String, u32>,
    speaker_count: u32,
    pub names: NameBook,
    sessions: Vec<Session>,
    selves_lower: Vec<String>,
    dedup: Deduper,
    /// When false (history loading) de-duplication runs after sorting instead.
    live_dedup: bool,
    next_id: u32,
    out: Batch,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Pipeline {
    pub fn new(rules: Vec<CompiledRule>) -> Self {
        Pipeline {
            classifier: Classifier::new(),
            rules,
            speakers: FxHashMap::default(),
            speaker_count: 0,
            names: NameBook::default(),
            sessions: Vec::new(),
            selves_lower: Vec::new(),
            dedup: Deduper::default(),
            live_dedup: true,
            next_id: 0,
            out: Batch {
                reset: true,
                ..Default::default()
            },
        }
    }

    pub fn set_rules(&mut self, rules: Vec<CompiledRule>) {
        self.rules = rules;
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn entry_count(&self) -> u32 {
        self.next_id
    }

    /// Begin a new file. `file` is a caller-chosen id unique per path.
    pub fn open_file(&mut self, file: u16, path: PathBuf) -> FileCursor {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_date = parse::file_name_stamp(&name).map(|(y, m, d, ..)| (y, m, d));
        let session = self.new_session(file, &name, &path, None);
        FileCursor {
            file,
            name,
            path,
            stamps: StampParser::new(file_date),
            session,
            self_name: None,
            staff_body: false,
            recent: VecDeque::with_capacity(8),
            last: None,
        }
    }

    fn new_session(
        &mut self,
        file: u16,
        name: &str,
        path: &std::path::Path,
        character: Option<String>,
    ) -> u16 {
        let id = self.sessions.len().min(u16::MAX as usize) as u16;
        let s = Session {
            file,
            file_name: name.to_string(),
            path: path.to_path_buf(),
            character,
        };
        if (id as usize) < self.sessions.len() {
            // Out of ids (65k sessions); reuse the last slot.
            self.sessions[id as usize] = s.clone();
        } else {
            self.sessions.push(s.clone());
        }
        self.out.sessions.push((id, s));
        id
    }

    fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.speakers.get(name) {
            return id;
        }
        let id = self.speaker_count;
        self.speaker_count += 1;
        self.speakers.insert(name.to_string(), id);
        self.out.new_speakers.push(name.to_string());
        id
    }

    fn push(
        &mut self,
        time: u32,
        session: u16,
        channel: Channel,
        entry_flags: u8,
        speaker: u32,
        text: &str,
    ) -> u32 {
        let off = self.out.text.len() as u32;
        self.out.text.push_str(text);
        push_lower(&mut self.out.lower, text);
        if self.out.entries.is_empty() {
            self.out.first_id = self.next_id;
        }
        self.out.entries.push(Entry {
            text_off: off,
            text_len: text.len() as u32,
            speaker,
            time,
            session,
            channel,
            flags: entry_flags,
            // Live lines get the second they arrived; history has minutes only.
            secs: if self.live_dedup {
                now_second()
            } else {
                crate::model::SECS_UNKNOWN
            },
        });
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn reclassify(&mut self, id: u32, channel: Channel) {
        let first = self.out.first_id;
        if !self.out.entries.is_empty() && id >= first {
            if let Some(e) = self.out.entries.get_mut((id - first) as usize) {
                e.channel = channel;
                return;
            }
        }
        self.out.reclass.push((id, channel));
    }

    fn dedup_key(channel: Channel, speaker: u32, text: &str) -> u64 {
        let mut h = FxHasher::default();
        channel.hash(&mut h);
        speaker.hash(&mut h);
        text.hash(&mut h);
        h.finish()
    }

    fn dedup_eligible(channel: Channel) -> bool {
        channel.is_chat() || matches!(channel, Channel::World | Channel::Spell)
    }

    /// Feed one raw line (without the line terminator).
    pub fn process_line(&mut self, fc: &mut FileCursor, line: &str) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return;
        }
        let Some(raw) = parse::split_line(line) else {
            // Continuation of a multi-line message: inherit the previous line.
            let (time, channel, speaker, fl) = match fc.last {
                Some(l) => l,
                None => (0, Channel::System, self.intern("System"), 0),
            };
            self.push(
                time,
                fc.session,
                channel,
                (fl & !flags::DUP) | flags::CONT,
                speaker,
                line,
            );
            return;
        };
        let time = fc
            .stamps
            .parse(raw.stamp)
            .or(fc.last.map(|l| l.0))
            .unwrap_or(0);

        // Character detection: "Welcome X!"
        if raw.name == "System" {
            if let Some(who) = raw
                .text
                .strip_prefix("Welcome ")
                .and_then(|r| r.strip_suffix('!'))
            {
                if !who.is_empty()
                    && who.len() <= 32
                    && who.split(' ').count() <= 3
                    && !who.starts_with("to ")
                {
                    self.set_character(fc, who, time);
                }
            }
        }

        let names = &self.names;
        let is_npc = |n: &str| names.kind(n) == PersonKind::Npc;
        let is_pet = |n: &str| names.kind(n) == PersonKind::Pet;
        let ctx = Ctx {
            self_name: fc.self_name.as_deref(),
            staff_body: fc.staff_body,
            is_npc: &is_npc,
            is_pet: &is_pet,
        };
        let mut c = self.classifier.classify(raw.name, raw.text, &ctx);
        fc.staff_body = raw.name == "System" && raw.text.starts_with("Staff message from ");

        if c.label.is_none() {
            for rule in &self.rules {
                if let Some(ch) = rule.apply(c.speaker, raw.text, c.channel) {
                    c.channel = ch;
                    break;
                }
            }
        }

        // Mentions of one of your characters in chat.
        if c.channel.is_chat() && c.flags & flags::SELF == 0 && !self.selves_lower.is_empty() {
            let mut lower = String::with_capacity(raw.text.len());
            push_lower(&mut lower, raw.text);
            if self.selves_lower.iter().any(|s| contains_word(&lower, s)) {
                c.flags |= flags::MENTION;
            }
        }

        let speaker = self.intern(c.speaker);
        let mut entry_flags = c.flags;
        if self.live_dedup && Self::dedup_eligible(c.channel) {
            let key = Self::dedup_key(c.channel, speaker, raw.text);
            if self.dedup.check(key, time, fc.file) {
                entry_flags |= flags::DUP;
            }
        }
        let id = self.push(time, fc.session, c.channel, entry_flags, speaker, raw.text);
        fc.last = Some((time, c.channel, speaker, c.flags));

        // Name registry + label bursts.
        match c.label {
            Some(label) => {
                match label {
                    Label::FullName => self.names.saw_label(c.speaker, time),
                    Label::Titled { title } => {
                        self.names.saw_label(c.speaker, time);
                        self.names.saw_title(c.speaker, title, time);
                    }
                    Label::Npc { title } => {
                        self.names.saw_label(c.speaker, time);
                        self.names.saw_npc_title(c.speaker, title, time);
                    }
                    Label::GuildTag { inner } => self.names.saw_guild_tag(c.speaker, inner, time),
                    Label::Status { status } => self.names.saw_status(c.speaker, status, time),
                }
                if label.ends_burst() {
                    self.fix_burst(fc, speaker, time, c.speaker);
                }
            }
            None => match c.channel {
                Channel::Guild | Channel::Alliance | Channel::Party => {
                    self.names.saw_speech(c.speaker, true, time);
                    if let Some(g) = c.chat_guild {
                        self.names.saw_guild_abbr(c.speaker, g, time);
                    }
                }
                Channel::Speech | Channel::Npc | Channel::Emote | Channel::Spell => {
                    self.names.saw_speech(c.speaker, false, time)
                }
                Channel::Combat => {
                    if let Some(n) = c.amount {
                        if raw.name != "System" {
                            self.names.saw_combat(c.speaker, n, time);
                        }
                    }
                }
                _ => {}
            },
        }

        if fc.recent.len() == 8 {
            fc.recent.pop_front();
        }
        fc.recent.push_back(Recent {
            id,
            speaker,
            time,
            channel: c.channel,
            title_like: c.label.is_none() && looks_like_title(raw.text),
        });
    }

    /// A name label just ended a burst like
    /// `Seasoned Angler` / `[Steward, OAK]` / `Corwen Ash`: move the title lines
    /// in front of it (classified as speech when they arrived) to Names.
    fn fix_burst(&mut self, fc: &mut FileCursor, speaker: u32, time: u32, name: &str) {
        let mut fixed = Vec::new();
        // `recent` does not contain the label line itself yet.
        for r in fc.recent.iter().rev().take(3) {
            if r.speaker != speaker || r.time.abs_diff(time) > 1 {
                break;
            }
            if r.channel == Channel::Names {
                continue;
            }
            if r.title_like && matches!(r.channel, Channel::Speech | Channel::Npc | Channel::Emote)
            {
                fixed.push(r.id);
            } else {
                break;
            }
        }
        for id in fixed {
            self.reclassify(id, Channel::Names);
            if let Some(r) = fc.recent.iter_mut().find(|r| r.id == id) {
                r.channel = Channel::Names;
            }
            // The title text is only in the arena of the current batch, if at all.
            if let Some(title) = self.entry_text(id) {
                let title = title.to_string();
                self.names.saw_title(name, &title, time);
            }
        }
    }

    fn entry_text(&self, id: u32) -> Option<&str> {
        let first = self.out.first_id;
        let e = self.out.entries.get(id.checked_sub(first)? as usize)?;
        self.out
            .text
            .get(e.text_off as usize..(e.text_off + e.text_len) as usize)
    }

    fn set_character(&mut self, fc: &mut FileCursor, who: &str, time: u32) {
        let lower = who.to_lowercase();
        if !self.selves_lower.contains(&lower) {
            self.selves_lower.push(lower);
        }
        self.names.mark_self(who, time);
        if fc.self_name.as_deref() == Some(who) {
            return;
        }
        let current = &mut self.sessions[fc.session as usize];
        if current.character.is_none() {
            current.character = Some(who.to_string());
            let s = current.clone();
            self.out.sessions.push((fc.session, s));
        } else {
            fc.session = self.new_session(
                fc.file,
                &fc.name.clone(),
                &fc.path.clone(),
                Some(who.to_string()),
            );
        }
        fc.self_name = Some(who.to_string());
    }

    /// Feed a whole chunk of text (several lines). A trailing partial line must not
    /// be included – the caller keeps it until its newline arrives.
    pub fn process_chunk(&mut self, fc: &mut FileCursor, chunk: &str) {
        for line in chunk.split('\n') {
            self.process_line(fc, line);
        }
    }

    /// Switch to bulk mode for loading history: entries are later sorted by time
    /// and de-duplicated in one pass by [`Pipeline::finish_history`].
    pub fn begin_history(&mut self) {
        self.live_dedup = false;
    }

    /// Sort the history batch by time (stable, so per-file order is kept within a
    /// minute), run de-duplication and return to live mode.
    /// Call [`FileCursor::forget_recent`] on every cursor afterwards: entry ids
    /// remembered for label fix-ups are invalid once the batch is sorted.
    pub fn finish_history(&mut self) {
        self.live_dedup = true;
        // Entries were pushed file by file; ids are positions, so sorting is safe
        // as long as nothing outside this batch refers to them yet.
        debug_assert!(self.out.reclass.is_empty());
        self.out.entries.sort_by_key(|e| e.time);
        self.dedup.clear();
        let text = &self.out.text;
        for e in self.out.entries.iter_mut() {
            if !Self::dedup_eligible(e.channel) {
                continue;
            }
            let file = self
                .sessions
                .get(e.session as usize)
                .map(|s| s.file)
                .unwrap_or(0);
            let t = &text[e.text_off as usize..(e.text_off + e.text_len) as usize];
            if self
                .dedup
                .check(Self::dedup_key(e.channel, e.speaker, t), e.time, file)
            {
                e.flags |= flags::DUP;
            }
        }
    }

    /// Take everything produced since the last call.
    pub fn take_batch(&mut self) -> Batch {
        let mut b = std::mem::take(&mut self.out);
        b.people = self.names.take_dirty();
        self.out.first_id = self.next_id;
        b
    }

    /// Forget everything; the next batch tells the store to reset.
    pub fn reset(&mut self) {
        let rules = std::mem::take(&mut self.rules);
        *self = Pipeline::new(rules);
    }
}

/// Seconds part of the current time (same in every timezone).
fn now_second() -> u8 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() % 60) as u8)
        .unwrap_or(crate::model::SECS_UNKNOWN)
}

/// Word-ish containment: `needle` must not be glued to letters on either side.
fn contains_word(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hb = hay.as_bytes();
    let mut start = 0;
    while let Some(pos) = memchr::memmem::find(&hb[start..], needle.as_bytes()) {
        let i = start + pos;
        let j = i + needle.len();
        let before = i == 0 || !hb[i - 1].is_ascii_alphanumeric();
        let after = j >= hb.len() || !hb[j].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        start = i + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // All names, tags and chat below are invented.

    fn run(lines: &str) -> (Pipeline, Batch) {
        let mut p = Pipeline::default();
        let mut fc = p.open_file(0, PathBuf::from("2026_03_14_18_02_11_journal.txt"));
        p.process_chunk(&mut fc, lines);
        let b = p.take_batch();
        (p, b)
    }

    fn text(b: &Batch, i: usize) -> &str {
        let e = &b.entries[i];
        &b.text[e.text_off as usize..(e.text_off + e.text_len) as usize]
    }

    #[test]
    fn welcome_and_bursts() {
        let log = "[03/14/2026 18:02]  System: Welcome Aldric Thorne!\r\n\
[03/14/2026 18:02]  Aldric Thorne: [OAK]\r\n\
[03/14/2026 18:02]  Aldric Thorne: Aldric Thorne\r\n\
[03/14/2026 18:03]  Corwen Ash: Seasoned Angler\r\n\
[03/14/2026 18:03]  Corwen Ash: Corwen Ash\r\n\
[03/14/2026 18:03]  Garrick: Garrick the tanner\r\n\
[03/14/2026 18:03]  Garrick: Fine hides, cheap!\r\n\
[03/14/2026 18:03]  Aldric Thorne: Kal Ort Por [Recall]\r\n\
[03/14/2026 18:03]  [Guild][Oswin Pike]: aldric thorne you there?\r\n";
        let (p, b) = run(log);
        assert!(b.reset);
        assert_eq!(b.entries.len(), 9);
        assert_eq!(
            b.sessions.last().unwrap().1.character.as_deref(),
            Some("Aldric Thorne")
        );
        let chans: Vec<Channel> = b.entries.iter().map(|e| e.channel).collect();
        assert_eq!(
            chans,
            vec![
                Channel::System,
                Channel::Names,
                Channel::Names,
                Channel::Names, // title line fixed up by the burst
                Channel::Names,
                Channel::Names,
                Channel::Npc,
                Channel::Spell,
                Channel::Guild,
            ]
        );
        assert!(b.entries[7].has(flags::SELF));
        assert!(b.entries[8].has(flags::MENTION));
        assert_eq!(text(&b, 3), "Seasoned Angler");
        let corwen = p.names.get("Corwen Ash").unwrap();
        assert_eq!(corwen.title.as_deref(), Some("Seasoned Angler"));
        assert_eq!(
            p.names.get("Aldric Thorne").unwrap().guild.as_deref(),
            Some("OAK")
        );
    }

    #[test]
    fn burst_fix_across_batches() {
        let mut p = Pipeline::default();
        let mut fc = p.open_file(0, PathBuf::from("2026_03_14_18_02_11_journal.txt"));
        p.process_chunk(&mut fc, "[03/14/2026 18:03]  Bramblewick: Master Carpenter");
        let b1 = p.take_batch();
        assert_eq!(b1.entries[0].channel, Channel::Speech);
        p.process_chunk(&mut fc, "[03/14/2026 18:03]  Bramblewick: Bramblewick");
        let b2 = p.take_batch();
        assert_eq!(b2.first_id, 1);
        assert_eq!(b2.reclass, vec![(0, Channel::Names)]);
    }

    #[test]
    fn continuation_lines() {
        let (_, b) = run("[03/14/2026 18:10]  System: Staff message from Warden Hesper:\n[03/14/2026 18:10]  System: Harvestide has begun!\nsecond line\n");
        assert_eq!(b.entries.len(), 3);
        assert_eq!(b.entries[1].channel, Channel::World);
        assert!(b.entries[2].has(flags::CONT));
        assert_eq!(b.entries[2].channel, Channel::World);
    }

    #[test]
    fn dedup_across_files() {
        let mut p = Pipeline::default();
        let mut a = p.open_file(0, PathBuf::from("2026_03_14_18_02_11_journal.txt"));
        let mut b = p.open_file(1, PathBuf::from("2026_03_14_18_05_40_journal.txt"));
        let line = "[03/14/2026 18:12]  [Alliance][Lysa Quill]: [OAK] reds at the north bridge";
        p.process_line(&mut a, line);
        p.process_line(&mut b, line);
        p.process_line(&mut a, line); // same client repeating: not a dup
        let batch = p.take_batch();
        let dups: Vec<bool> = batch.entries.iter().map(|e| e.has(flags::DUP)).collect();
        assert_eq!(dups, vec![false, true, false]);
    }

    #[test]
    fn lower_keeps_layout() {
        let mut s = String::new();
        push_lower(&mut s, "Ärger İstanbul ẞ OK");
        assert_eq!(s.len(), "Ärger İstanbul ẞ OK".len());
        assert!(s.starts_with("ärger"));
        assert!(s.ends_with("ok"));
    }

    #[test]
    fn word_match() {
        assert!(contains_word("hey pip, there?", "pip"));
        assert!(!contains_word("pipkin", "pip"));
    }
}
