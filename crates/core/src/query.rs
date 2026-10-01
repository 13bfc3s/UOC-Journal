//! Search mini-language and incremental filtered views.
//!
//! ```text
//! red pk              both words (substring, case-insensitive)
//! Red                 smart case: an upper-case letter makes the term case-sensitive
//! "level 3 gate"      phrase
//! red|pk|gank         any of the alternatives (also: `red OR pk`)
//! -world              exclude
//! from:quill         speaker contains   (also @quill, from:"oswin pike")
//! ch:guild,ally       channel            (also in:/channel:)
//! char:thorne         character whose journal it is
//! is:self is:mention is:dup is:incoming
//! /\d+ reds?/         regular expression (smart case)
//! ```

use std::ops::Range;

use memchr::memmem::Finder;
use regex::Regex;

use crate::model::{flags, Channel, ChannelSet, Entry};
use crate::store::Store;

enum Atom {
    /// Needle matched against lower-cased text and speaker.
    Lower(Finder<'static>),
    /// Needle with upper-case letters matched against original text and speaker.
    Exact(Finder<'static>),
    Regex(Regex),
    From(Finder<'static>),
    Channel(ChannelSet),
    Char(Finder<'static>),
    Flag(u8),
}

struct Clause {
    negate: bool,
    alts: Vec<Atom>,
}

/// A parsed search expression. An empty query matches everything.
#[derive(Default)]
pub struct Query {
    clauses: Vec<Clause>,
    source: String,
    error: Option<String>,
}

impl Clone for Query {
    fn clone(&self) -> Self {
        Query::parse(&self.source)
    }
}

impl std::fmt::Debug for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Query({:?})", self.source)
    }
}

struct Ctx<'a> {
    text: &'a str,
    lower: &'a str,
    speaker: &'a str,
    speaker_lower: &'a str,
    character_lower: &'a str,
    entry: &'a Entry,
}

fn has_upper(s: &str) -> bool {
    s.chars().any(|c| c.is_uppercase())
}

fn lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    crate::pipeline::push_lower(&mut out, s);
    out
}

/// Split the input into raw tokens, respecting double quotes and `/regex/`.
fn tokenize(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut tok = String::new();
        // Regex token (optionally negated) runs to the closing slash.
        let neg = chars[i] == '-' && chars.get(i + 1) == Some(&'/');
        if chars[i] == '/' || neg {
            if neg {
                tok.push('-');
                i += 1;
            }
            tok.push('/');
            i += 1;
            while i < chars.len() {
                let c = chars[i];
                if c == '\\' && i + 1 < chars.len() {
                    tok.push(c);
                    tok.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                tok.push(c);
                i += 1;
                if c == '/' {
                    break;
                }
            }
            out.push(tok);
            continue;
        }
        let mut in_quote = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                in_quote = !in_quote;
                tok.push(c);
                i += 1;
                continue;
            }
            if c.is_whitespace() && !in_quote {
                break;
            }
            tok.push(c);
            i += 1;
        }
        out.push(tok);
    }
    out
}

/// Split on `|` outside of quotes.
fn split_alts(tok: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in tok.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '|' if !in_quote => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .map(|r| r.strip_suffix('"').unwrap_or(r))
        .unwrap_or(s)
}

fn text_atom(needle: &str) -> Option<Atom> {
    if needle.is_empty() {
        return None;
    }
    Some(if has_upper(needle) {
        Atom::Exact(Finder::new(needle.as_bytes()).into_owned())
    } else {
        Atom::Lower(Finder::new(lower(needle).as_bytes()).into_owned())
    })
}

fn parse_atom(tok: &str) -> Result<Option<Atom>, String> {
    if tok.len() >= 2 && tok.starts_with('/') {
        let body = tok[1..].strip_suffix('/').unwrap_or(&tok[1..]);
        if body.is_empty() {
            return Ok(None);
        }
        let pat = if has_upper(body) {
            body.to_string()
        } else {
            format!("(?i){body}")
        };
        return Regex::new(&pat)
            .map(|r| Some(Atom::Regex(r)))
            .map_err(|e| e.to_string());
    }
    if let Some(name) = tok.strip_prefix('@') {
        let name = unquote(name);
        return Ok((!name.is_empty())
            .then(|| Atom::From(Finder::new(lower(name).as_bytes()).into_owned())));
    }
    if let Some((key, value)) = tok.split_once(':') {
        let value = unquote(value);
        match key.to_ascii_lowercase().as_str() {
            "from" | "by" | "who" => {
                return Ok((!value.is_empty())
                    .then(|| Atom::From(Finder::new(lower(value).as_bytes()).into_owned())));
            }
            "ch" | "in" | "channel" | "chan" => {
                let mut set = ChannelSet::EMPTY;
                for part in value.split(',').filter(|p| !p.is_empty()) {
                    match Channel::parse(part) {
                        Some(c) => set.insert(c),
                        None => return Err(format!("unknown channel '{part}'")),
                    }
                }
                return Ok((!set.is_empty()).then_some(Atom::Channel(set)));
            }
            "char" | "character" | "me" => {
                return Ok((!value.is_empty())
                    .then(|| Atom::Char(Finder::new(lower(value).as_bytes()).into_owned())));
            }
            "is" | "has" => {
                let f = match value.to_ascii_lowercase().as_str() {
                    "self" | "me" | "mine" => flags::SELF,
                    "mention" | "mentions" => flags::MENTION,
                    "dup" | "duplicate" => flags::DUP,
                    "incoming" | "in" | "taken" => flags::INCOMING,
                    other => return Err(format!("unknown flag 'is:{other}'")),
                };
                return Ok(Some(Atom::Flag(f)));
            }
            "re" | "regex" => return parse_atom(&format!("/{value}/")),
            _ => {} // plain text that happens to contain a colon
        }
    }
    Ok(text_atom(unquote(tok)))
}

impl Query {
    pub fn parse(src: &str) -> Query {
        let mut q = Query {
            clauses: Vec::new(),
            source: src.to_string(),
            error: None,
        };
        let mut pending_or = false;
        for tok in tokenize(src) {
            if tok == "OR" || tok == "|" {
                pending_or = true;
                continue;
            }
            let (negate, body) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, tok.as_str()),
            };
            let mut alts = Vec::new();
            for alt in split_alts(body) {
                match parse_atom(&alt) {
                    Ok(Some(a)) => alts.push(a),
                    Ok(None) => {}
                    Err(e) => {
                        q.error.get_or_insert(e);
                    }
                }
            }
            if alts.is_empty() {
                continue;
            }
            match q.clauses.last_mut() {
                Some(last) if pending_or && !negate && !last.negate => last.alts.extend(alts),
                _ => q.clauses.push(Clause { negate, alts }),
            }
            pending_or = false;
        }
        q
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    fn atom_matches(atom: &Atom, c: &Ctx<'_>) -> bool {
        match atom {
            Atom::Lower(f) => {
                f.find(c.lower.as_bytes()).is_some() || f.find(c.speaker_lower.as_bytes()).is_some()
            }
            Atom::Exact(f) => {
                f.find(c.text.as_bytes()).is_some() || f.find(c.speaker.as_bytes()).is_some()
            }
            Atom::Regex(r) => r.is_match(c.text) || r.is_match(c.speaker),
            Atom::From(f) => f.find(c.speaker_lower.as_bytes()).is_some(),
            Atom::Channel(set) => set.contains(c.entry.channel),
            Atom::Char(f) => f.find(c.character_lower.as_bytes()).is_some(),
            Atom::Flag(fl) => c.entry.flags & fl != 0,
        }
    }

    fn matches_ctx(&self, c: &Ctx<'_>) -> bool {
        self.clauses
            .iter()
            .all(|cl| cl.alts.iter().any(|a| Self::atom_matches(a, c)) != cl.negate)
    }

    /// Does entry `e` match this query?
    pub fn matches(&self, store: &Store, e: &Entry) -> bool {
        if self.clauses.is_empty() {
            return true;
        }
        let chars = store.session_chars_lower();
        let ctx = Ctx {
            text: store.text(e),
            lower: store.lower(e),
            speaker: store.speaker(e),
            speaker_lower: store.speaker_lower(e),
            character_lower: chars
                .get(e.session as usize)
                .map(String::as_str)
                .unwrap_or(""),
            entry: e,
        };
        self.matches_ctx(&ctx)
    }

    /// Byte ranges of `text` that positive text/regex terms matched (for highlighting).
    pub fn highlights(&self, text: &str, lower_text: &str, out: &mut Vec<Range<usize>>) {
        for cl in self.clauses.iter().filter(|c| !c.negate) {
            for a in &cl.alts {
                match a {
                    Atom::Lower(f) => push_all(f, lower_text.as_bytes(), out),
                    Atom::Exact(f) => push_all(f, text.as_bytes(), out),
                    Atom::Regex(r) => out.extend(
                        r.find_iter(text)
                            .filter(|m| !m.is_empty())
                            .map(|m| m.range()),
                    ),
                    _ => {}
                }
            }
        }
        merge_ranges(out);
        // Never split a UTF-8 character.
        out.retain(|r| text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
    }
}

fn push_all(f: &Finder<'_>, hay: &[u8], out: &mut Vec<Range<usize>>) {
    let n = f.needle().len();
    if n == 0 {
        return;
    }
    let mut start = 0;
    while let Some(i) = f.find(&hay[start..]) {
        out.push(start + i..start + i + n);
        start += i + n;
        if out.len() > 256 {
            break;
        }
    }
}

pub fn merge_ranges(v: &mut Vec<Range<usize>>) {
    if v.len() < 2 {
        return;
    }
    v.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(v.len());
    for r in v.drain(..) {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    *v = out;
}

/// Everything that decides whether a row shows up in a pane.
#[derive(Clone, Debug)]
pub struct Filter {
    pub channels: ChannelSet,
    /// Lower-case character names; empty = all characters.
    pub characters: Vec<String>,
    /// Show messages already received by another client.
    pub show_dups: bool,
    /// The pane's saved filter expression.
    pub base: Query,
    /// The live search box.
    pub search: Query,
    /// Ignore entries with a lower id ("clear journal" without losing data).
    pub min_id: u32,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            channels: ChannelSet::ALL,
            characters: Vec::new(),
            show_dups: false,
            base: Query::default(),
            search: Query::default(),
            min_id: 0,
        }
    }
}

impl Filter {
    fn session_mask(&self, store: &Store) -> Option<Vec<bool>> {
        if self.characters.is_empty() {
            return None;
        }
        Some(
            store
                .session_chars_lower()
                .iter()
                .map(|c| self.characters.iter().any(|w| w == c))
                .collect(),
        )
    }

    #[inline]
    fn accepts(&self, store: &Store, e: &Entry, mask: Option<&[bool]>) -> bool {
        if !self.channels.contains(e.channel) {
            return false;
        }
        if !self.show_dups && e.flags & flags::DUP != 0 {
            return false;
        }
        if let Some(m) = mask {
            if !m.get(e.session as usize).copied().unwrap_or(false) {
                return false;
            }
        }
        self.base.matches(store, e) && self.search.matches(store, e)
    }
}

/// Ids of the store entries that pass a filter, maintained incrementally.
#[derive(Default, Debug)]
pub struct View {
    pub rows: Vec<u32>,
    scanned: usize,
    epoch: u64,
    reclass_seen: usize,
}

/// What [`View::sync`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncResult {
    /// Rows were dropped from the end or the view was rebuilt.
    pub rebuilt: bool,
    pub appended: usize,
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    /// Force a full rescan at the next sync (filter changed).
    pub fn invalidate(&mut self) {
        self.rows.clear();
        self.scanned = 0;
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn sync(&mut self, store: &Store, filter: &Filter) -> SyncResult {
        let mut res = SyncResult::default();
        if self.epoch != store.epoch() {
            self.epoch = store.epoch();
            self.reclass_seen = 0;
            self.invalidate();
            res.rebuilt = true;
        }
        let log = store.reclass_log();
        if self.reclass_seen < log.len() {
            let min = log[self.reclass_seen..]
                .iter()
                .copied()
                .min()
                .unwrap_or(u32::MAX) as usize;
            self.reclass_seen = log.len();
            if min < self.scanned {
                let cut = self.rows.partition_point(|&id| (id as usize) < min);
                if cut < self.rows.len() {
                    res.rebuilt = true;
                }
                self.rows.truncate(cut);
                self.scanned = min;
            }
        }
        let total = store.len();
        if self.scanned >= total {
            return res;
        }
        if self.scanned == 0 && !self.rows.is_empty() {
            self.rows.clear();
        }
        let mask = filter.session_mask(store);
        let before = self.rows.len();
        let start = self.scanned.max(filter.min_id as usize).min(total);
        let entries = store.entries();
        let n = total - start;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        if n > 150_000 && threads > 1 {
            let chunk = n.div_ceil(threads);
            let parts: Vec<Vec<u32>> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..threads)
                    .map(|t| {
                        let lo = start + t * chunk;
                        let hi = (lo + chunk).min(total);
                        let mask = mask.as_deref();
                        s.spawn(move || {
                            (lo..hi)
                                .filter(|&i| filter.accepts(store, &entries[i], mask))
                                .map(|i| i as u32)
                                .collect::<Vec<u32>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect()
            });
            for p in parts {
                self.rows.extend(p);
            }
        } else {
            let mask = mask.as_deref();
            for (i, e) in entries[start..].iter().enumerate() {
                if filter.accepts(store, e, mask) {
                    self.rows.push((start + i) as u32);
                }
            }
        }
        self.scanned = total;
        res.appended = self.rows.len() - before;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Pipeline;
    use std::path::PathBuf;

    // All names, tags and chat below are invented.
    const LOG: &str = "[03/14/2026 18:02]  System: Welcome Aldric Thorne!
[03/14/2026 18:12]  [Alliance][Lysa Quill]: [OAK] reds near the north bridge
[03/14/2026 18:13]  [Guild][Oswin Pike]: Hesper did you check the mines
[03/14/2026 18:14]  [Alliance][Corwen Ash]: [VEX] 1 red in mines L2
[03/14/2026 18:20]  System: The world is saving, please wait.
[03/14/2026 18:21]  a frost troll: -210
[03/14/2026 18:21]  Aldric Thorne: -25
[03/14/2026 18:22]  [Guild][Hesper Nightjar]: aldric thorne where are you
";

    fn store() -> Store {
        let mut p = Pipeline::default();
        let mut fc = p.open_file(0, PathBuf::from("2026_03_14_18_02_11_journal.txt"));
        p.process_chunk(&mut fc, LOG);
        let mut s = Store::new();
        assert!(s.apply(p.take_batch()));
        s
    }

    fn ids(store: &Store, q: &str) -> Vec<u32> {
        let f = Filter {
            search: Query::parse(q),
            ..Default::default()
        };
        let mut v = View::new();
        v.sync(store, &f);
        v.rows
    }

    #[test]
    fn queries() {
        let s = store();
        assert_eq!(ids(&s, ""), (0..8).collect::<Vec<_>>());
        assert_eq!(ids(&s, "red"), vec![1, 3]);
        assert_eq!(ids(&s, "reds|hesper"), vec![1, 2, 7]);
        assert_eq!(ids(&s, "reds OR hesper"), vec![1, 2, 7]);
        assert_eq!(ids(&s, "red -bridge"), vec![3]);
        assert_eq!(ids(&s, "from:quill"), vec![1]);
        assert_eq!(ids(&s, "@\"oswin pike\""), vec![2]);
        assert_eq!(ids(&s, "ch:guild"), vec![2, 7]);
        assert_eq!(ids(&s, "ch:guild,ally"), vec![1, 2, 3, 7]);
        assert_eq!(ids(&s, "\"world is saving\""), vec![4]);
        assert_eq!(ids(&s, "/\\d+ reds?/"), vec![3]);
        assert_eq!(ids(&s, "Hesper"), vec![2, 7]);
        assert_eq!(ids(&s, "HESPER"), Vec::<u32>::new());
        assert_eq!(ids(&s, "is:mention"), vec![7]);
        assert_eq!(ids(&s, "is:incoming"), vec![6]);
        assert_eq!(ids(&s, "char:thorne ch:combat"), vec![5, 6]);
        assert!(Query::parse("ch:nope").error().is_some());
        assert!(Query::parse("/(/").error().is_some());
        assert_eq!(ids(&s, "mines l2"), vec![3]);
    }

    #[test]
    fn highlights() {
        let q = Query::parse("red l2");
        let mut out = Vec::new();
        q.highlights("[VEX] 1 Red mines L2", "[vex] 1 red mines l2", &mut out);
        assert_eq!(out, vec![8..11, 18..20]);
    }

    #[test]
    fn incremental_view() {
        let s = store();
        let f = Filter {
            channels: ChannelSet::of(&[Channel::Guild]),
            ..Default::default()
        };
        let mut v = View::new();
        let r = v.sync(&s, &f);
        assert_eq!(r.appended, 2);
        assert_eq!(v.sync(&s, &f).appended, 0);
    }
}
