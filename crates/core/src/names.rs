//! Registry of everyone whose name has been shown or who has spoken.

use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum PersonKind {
    #[default]
    Unknown,
    Player,
    Npc,
    Pet,
    Creature,
}

impl PersonKind {
    pub fn label(self) -> &'static str {
        match self {
            PersonKind::Unknown => "?",
            PersonKind::Player => "Player",
            PersonKind::Npc => "NPC",
            PersonKind::Pet => "Pet",
            PersonKind::Creature => "Creature",
        }
    }

    /// How sure we are about a kind; stronger evidence wins.
    fn strength(self) -> u8 {
        match self {
            PersonKind::Unknown => 0,
            PersonKind::Creature => 1,
            PersonKind::Npc => 2,
            PersonKind::Pet => 3,
            PersonKind::Player => 4,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub kind: PersonKind,
    /// Guild abbreviation, e.g. `OAK`.
    pub guild: Option<String>,
    /// Guild rank/title, e.g. `Steward` from `[Steward, OAK]`.
    pub guild_title: Option<String>,
    /// Display titles: `Lord`, `Seasoned Angler`, `the tanner`, ...
    pub title: Option<String>,
    /// Pet status such as `bonded`.
    pub status: Option<String>,
    pub first_seen: u32,
    pub last_seen: u32,
    /// Times a name label was shown.
    pub shown: u32,
    /// Lines spoken (any chat channel).
    pub said: u32,
    /// Combat numbers on this mobile, summed (negative = damage).
    pub damage_taken: i64,
    /// One of your own characters.
    pub is_self: bool,
}

impl Person {
    fn new(name: &str, time: u32) -> Self {
        Person {
            name: name.to_string(),
            first_seen: time,
            last_seen: time,
            ..Default::default()
        }
    }
}

/// Name -> person. Changed names are tracked so the worker can ship deltas.
#[derive(Default)]
pub struct NameBook {
    map: FxHashMap<String, Person>,
    dirty: Vec<String>,
}

impl NameBook {
    pub fn get(&self, name: &str) -> Option<&Person> {
        self.map.get(name)
    }

    pub fn kind(&self, name: &str) -> PersonKind {
        self.map.get(name).map(|p| p.kind).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.dirty.clear();
    }

    fn touch(&mut self, name: &str, time: u32) -> &mut Person {
        if !self.map.contains_key(name) {
            self.map.insert(name.to_string(), Person::new(name, time));
        }
        if self.dirty.last().map(|d| d != name).unwrap_or(true) {
            self.dirty.push(name.to_string());
        }
        let p = self.map.get_mut(name).expect("just inserted");
        if time > p.last_seen {
            p.last_seen = time;
        }
        if time < p.first_seen {
            p.first_seen = time;
        }
        p
    }

    fn set_kind(p: &mut Person, kind: PersonKind) {
        if kind.strength() > p.kind.strength() {
            p.kind = kind;
        }
    }

    pub fn saw_label(&mut self, name: &str, time: u32) {
        let p = self.touch(name, time);
        p.shown += 1;
        if p.kind == PersonKind::Unknown && starts_with_article(name) {
            p.kind = PersonKind::Creature;
        }
    }

    pub fn saw_title(&mut self, name: &str, title: &str, time: u32) {
        let p = self.touch(name, time);
        if !title.is_empty() {
            p.title = Some(title.to_string());
        }
        if matches!(title, "Lord" | "Lady") {
            Self::set_kind(p, PersonKind::Player);
        }
    }

    pub fn saw_npc_title(&mut self, name: &str, title: &str, time: u32) {
        let p = self.touch(name, time);
        p.title = Some(title.to_string());
        Self::set_kind(p, PersonKind::Npc);
    }

    /// `[Steward, OAK]` → guild OAK, rank Steward.
    pub fn saw_guild_tag(&mut self, name: &str, inner: &str, time: u32) {
        let p = self.touch(name, time);
        let mut parts = inner.rsplitn(2, ',');
        let abbr = parts.next().unwrap_or("").trim();
        let rank = parts.next().map(str::trim).filter(|s| !s.is_empty());
        if !abbr.is_empty() {
            p.guild = Some(abbr.to_string());
        }
        p.guild_title = rank.map(str::to_string);
        Self::set_kind(p, PersonKind::Player);
    }

    pub fn saw_guild_abbr(&mut self, name: &str, abbr: &str, time: u32) {
        let p = self.touch(name, time);
        if !abbr.is_empty() {
            p.guild = Some(abbr.to_string());
        }
        Self::set_kind(p, PersonKind::Player);
    }

    pub fn saw_status(&mut self, name: &str, status: &str, time: u32) {
        let p = self.touch(name, time);
        p.status = Some(status.to_string());
        let s = status.to_ascii_lowercase();
        if s.contains("bonded")
            || s.contains("tame")
            || s.contains("summoned")
            || s.contains("released")
        {
            Self::set_kind(p, PersonKind::Pet);
        }
    }

    pub fn saw_speech(&mut self, name: &str, player_channel: bool, time: u32) {
        let p = self.touch(name, time);
        p.said += 1;
        if player_channel {
            Self::set_kind(p, PersonKind::Player);
        }
    }

    pub fn saw_combat(&mut self, name: &str, amount: i64, time: u32) {
        let p = self.touch(name, time);
        p.damage_taken += amount;
        if p.kind == PersonKind::Unknown && starts_with_article(name) {
            p.kind = PersonKind::Creature;
        }
    }

    pub fn mark_self(&mut self, name: &str, time: u32) {
        let p = self.touch(name, time);
        p.is_self = true;
        Self::set_kind(p, PersonKind::Player);
    }

    /// Take the people that changed since the last call.
    pub fn take_dirty(&mut self) -> Vec<Person> {
        let mut names = std::mem::take(&mut self.dirty);
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter_map(|n| self.map.get(&n).cloned())
            .collect()
    }

    pub fn all(&self) -> impl Iterator<Item = &Person> {
        self.map.values()
    }
}

/// `a dire wolf`, `an ice serpent`, `the Pale Watcher` ...
pub fn starts_with_article(name: &str) -> bool {
    name.starts_with("a ") || name.starts_with("an ") || name.starts_with("the ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // All names and tags below are invented.

    #[test]
    fn guild_tags() {
        let mut b = NameBook::default();
        b.saw_guild_tag("Fennick Dale", "Steward, OAK", 5);
        b.saw_guild_tag("Aldric Thorne", "VEX", 5);
        let c = b.get("Fennick Dale").unwrap();
        assert_eq!(c.guild.as_deref(), Some("OAK"));
        assert_eq!(c.guild_title.as_deref(), Some("Steward"));
        assert_eq!(c.kind, PersonKind::Player);
        assert_eq!(
            b.get("Aldric Thorne").unwrap().guild.as_deref(),
            Some("VEX")
        );
        assert_eq!(b.take_dirty().len(), 2);
        assert!(b.take_dirty().is_empty());
    }

    #[test]
    fn kinds_only_strengthen() {
        let mut b = NameBook::default();
        b.saw_status("Smudge", "bonded", 1);
        b.saw_label("Smudge", 1);
        assert_eq!(b.kind("Smudge"), PersonKind::Pet);
        b.saw_npc_title("Garrick", "the tanner", 1);
        assert_eq!(b.kind("Garrick"), PersonKind::Npc);
        b.saw_label("a dire wolf", 1);
        assert_eq!(b.kind("a dire wolf"), PersonKind::Creature);
    }
}
