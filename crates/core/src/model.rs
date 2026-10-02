use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What kind of message a journal line is. Every line gets exactly one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Channel {
    /// Overhead speech from players (or speakers we can't identify).
    Speech = 0,
    /// Speech from NPCs we have identified (vendors, "Name the cook", ...).
    Npc,
    /// `*emotes*`.
    Emote,
    /// Spell power words (`Kal Ort Por [Recall]`).
    Spell,
    Guild,
    Alliance,
    Party,
    /// Damage numbers, poison, parries, bandages, creature combat emotes, ...
    Combat,
    /// Skill gains/losses and skill-use notices.
    Skill,
    /// Personal system messages.
    System,
    /// Server-wide broadcasts: world saves, achievements, events, staff messages.
    World,
    /// Shown names: name labels, titles, guild tags, pet status.
    Names,
    /// Item labels (`You see: ...`) and object messages.
    Items,
    /// Razor / assistant messages.
    Razor,
    /// Purely client-side noise (WorldMap loading, ...).
    Client,
}

impl Channel {
    pub const COUNT: usize = 15;
    pub const ALL: [Channel; Channel::COUNT] = [
        Channel::Speech,
        Channel::Npc,
        Channel::Emote,
        Channel::Spell,
        Channel::Guild,
        Channel::Alliance,
        Channel::Party,
        Channel::Combat,
        Channel::Skill,
        Channel::System,
        Channel::World,
        Channel::Names,
        Channel::Items,
        Channel::Razor,
        Channel::Client,
    ];

    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub fn from_index(i: usize) -> Option<Channel> {
        Channel::ALL.get(i).copied()
    }

    #[inline]
    pub fn bit(self) -> u32 {
        1u32 << (self as u32)
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Speech => "Speech",
            Channel::Npc => "NPC",
            Channel::Emote => "Emote",
            Channel::Spell => "Spell",
            Channel::Guild => "Guild",
            Channel::Alliance => "Alliance",
            Channel::Party => "Party",
            Channel::Combat => "Combat",
            Channel::Skill => "Skill",
            Channel::System => "System",
            Channel::World => "World",
            Channel::Names => "Names",
            Channel::Items => "Items",
            Channel::Razor => "Razor",
            Channel::Client => "Client",
        }
    }

    /// Short badge text shown in front of a line.
    pub fn badge(self) -> &'static str {
        match self {
            Channel::Speech => "SAY",
            Channel::Npc => "NPC",
            Channel::Emote => "EMO",
            Channel::Spell => "SPL",
            Channel::Guild => "GLD",
            Channel::Alliance => "ALY",
            Channel::Party => "PTY",
            Channel::Combat => "CBT",
            Channel::Skill => "SKL",
            Channel::System => "SYS",
            Channel::World => "WLD",
            Channel::Names => "NAM",
            Channel::Items => "ITM",
            Channel::Razor => "RZR",
            Channel::Client => "CLI",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Channel::Speech => "Overhead speech from players",
            Channel::Npc => "Speech from identified NPCs (vendors, townsfolk)",
            Channel::Emote => "*emotes*",
            Channel::Spell => "Spell power words",
            Channel::Guild => "[Guild] chat",
            Channel::Alliance => "[Alliance] chat",
            Channel::Party => "[Party] chat",
            Channel::Combat => "Damage numbers, poison, parries, healing, creature emotes",
            Channel::Skill => "Skill gains and skill notices",
            Channel::System => "Personal system messages",
            Channel::World => "World saves, achievements, events, staff broadcasts",
            Channel::Names => "Name labels, titles, guild tags, pet status",
            Channel::Items => "Item labels (You see ...)",
            Channel::Razor => "Razor / assistant messages",
            Channel::Client => "Client-side noise (WorldMap loading, ...)",
        }
    }

    /// Parse a channel name as typed by a user (`guild`, `ally`, `sys`, ...).
    pub fn parse(s: &str) -> Option<Channel> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "speech" | "say" | "chat" | "public" => Channel::Speech,
            "npc" | "npcs" | "vendor" => Channel::Npc,
            "emote" | "emotes" | "emo" => Channel::Emote,
            "spell" | "spells" | "spl" | "magic" => Channel::Spell,
            "guild" | "gld" | "g" => Channel::Guild,
            "alliance" | "ally" | "aly" | "a" => Channel::Alliance,
            "party" | "pty" | "p" => Channel::Party,
            "combat" | "cbt" | "dmg" | "damage" | "fight" => Channel::Combat,
            "skill" | "skills" | "skl" => Channel::Skill,
            "system" | "sys" => Channel::System,
            "world" | "wld" | "broadcast" | "server" => Channel::World,
            "names" | "name" | "nam" | "labels" => Channel::Names,
            "items" | "item" | "itm" | "look" => Channel::Items,
            "razor" | "rzr" | "assistant" => Channel::Razor,
            "client" | "cli" => Channel::Client,
            _ => return None,
        })
    }

    /// Channels that carry conversation (used for mentions and de-duplication).
    pub fn is_chat(self) -> bool {
        matches!(
            self,
            Channel::Speech
                | Channel::Npc
                | Channel::Emote
                | Channel::Guild
                | Channel::Alliance
                | Channel::Party
        )
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A set of channels as a bitmask. Serialized as a list of channel names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChannelSet(pub u32);

impl ChannelSet {
    pub const EMPTY: ChannelSet = ChannelSet(0);
    pub const ALL: ChannelSet = ChannelSet((1u32 << Channel::COUNT) - 1);

    pub fn of(channels: &[Channel]) -> ChannelSet {
        ChannelSet(channels.iter().fold(0, |acc, c| acc | c.bit()))
    }

    #[inline]
    pub fn contains(self, c: Channel) -> bool {
        self.0 & c.bit() != 0
    }

    #[inline]
    pub fn insert(&mut self, c: Channel) {
        self.0 |= c.bit();
    }

    #[inline]
    pub fn remove(&mut self, c: Channel) {
        self.0 &= !c.bit();
    }

    pub fn toggle(&mut self, c: Channel) {
        self.0 ^= c.bit();
    }

    pub fn set(&mut self, c: Channel, on: bool) {
        if on {
            self.insert(c)
        } else {
            self.remove(c)
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 & ChannelSet::ALL.0 == 0
    }

    pub fn is_all(self) -> bool {
        self.0 & ChannelSet::ALL.0 == ChannelSet::ALL.0
    }

    pub fn iter(self) -> impl Iterator<Item = Channel> {
        Channel::ALL.into_iter().filter(move |c| self.contains(*c))
    }

    pub fn len(self) -> usize {
        (self.0 & ChannelSet::ALL.0).count_ones() as usize
    }
}

impl std::fmt::Debug for ChannelSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Serialize for ChannelSet {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v: Vec<Channel> = self.iter().collect();
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ChannelSet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names: Vec<String> = Vec::deserialize(d)?;
        let mut set = ChannelSet::EMPTY;
        for n in names {
            if n.eq_ignore_ascii_case("all") {
                set = ChannelSet::ALL;
            } else if let Some(c) = Channel::parse(&n) {
                set.insert(c);
            }
        }
        Ok(set)
    }
}

/// Per-entry flag bits.
pub mod flags {
    /// The speaker is the character whose journal this is.
    pub const SELF: u8 = 1 << 0;
    /// The text mentions one of your characters.
    pub const MENTION: u8 = 1 << 1;
    /// Same message already received by another running client.
    pub const DUP: u8 = 1 << 2;
    /// Continuation of a multi-line message.
    pub const CONT: u8 = 1 << 3;
    /// Combat number on you or one of your pets (damage taken).
    pub const INCOMING: u8 = 1 << 4;
}

/// One journal line. Text lives in the [`crate::Store`] arenas; this is 24 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Byte offset of the message text in the store's text arena.
    pub text_off: u32,
    /// Byte length of the message text.
    pub text_len: u32,
    /// Interned speaker id (see [`crate::Store::speaker`]).
    pub speaker: u32,
    /// Local time, minutes since 1970-01-01 (see [`crate::time`]).
    pub time: u32,
    /// Session (journal file + character) this line came from.
    pub session: u16,
    pub channel: Channel,
    pub flags: u8,
    /// Second within the minute when the line arrived live, or [`SECS_UNKNOWN`]
    /// (journal files only record minutes).
    pub secs: u8,
}

/// `Entry::secs` value for lines whose seconds are not known.
pub const SECS_UNKNOWN: u8 = 255;

impl Entry {
    pub fn seconds(&self) -> Option<u8> {
        (self.secs < 60).then_some(self.secs)
    }

    #[inline]
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_index_roundtrip() {
        for (i, c) in Channel::ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
            assert_eq!(Channel::from_index(i), Some(*c));
            assert_eq!(Channel::parse(c.label()), Some(*c));
        }
    }

    #[test]
    fn set_ops() {
        let mut s = ChannelSet::of(&[Channel::Guild, Channel::Party]);
        assert!(s.contains(Channel::Guild));
        assert!(!s.contains(Channel::Alliance));
        s.toggle(Channel::Alliance);
        assert_eq!(s.len(), 3);
        assert!(ChannelSet::ALL.is_all());
        assert_eq!(std::mem::size_of::<Entry>(), 24);
    }
}
