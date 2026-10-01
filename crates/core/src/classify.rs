//! Heuristic classification of a single `name: text` pair.
//!
//! The journal file loses ClassicUO's message type and hue, but the client still
//! encodes a lot in the name column:
//!
//! | name                 | meaning                                   |
//! |----------------------|-------------------------------------------|
//! | `System`             | system message                            |
//! | `[Guild][X]`         | guild chat from X                         |
//! | `[Alliance][X]`      | alliance chat from X                      |
//! | `[Party][X]`         | party chat from X                         |
//! | `You see`            | label of an item you clicked              |
//! | *(empty)*            | label of an object without a valid parent |
//! | `[Razor]` / `Razor`  | assistant messages                        |
//! | anything else        | speech, emote, spell or label of a mobile |
//!
//! The rest is pattern work: damage numbers (`a frost troll: -210`), label shapes
//! (`Fennick Dale: Lord Fennick Dale`, `Garrick: Garrick the tanner`, `Pip: (bonded)`), spell
//! words, emotes and keyword sets for combat/skill/world system messages.

use std::cell::RefCell;

use regex::{Regex, RegexSet};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::model::{flags, Channel, ChannelSet};

/// What a classified line tells us about a mobile's name label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label<'a> {
    /// `X: X` – the plain name label (ends a label burst).
    FullName,
    /// `X: Lord X` – name with a prefix title (ends a label burst).
    Titled { title: &'a str },
    /// `X: X the cook` – NPC name with profession (ends a label burst).
    Npc { title: &'a str },
    /// `X: [Steward, OAK]`
    GuildTag { inner: &'a str },
    /// `X: (bonded)` / `X: *released*`
    Status { status: &'a str },
}

impl Label<'_> {
    /// Whether this label is the final line of a name-label burst.
    pub fn ends_burst(&self) -> bool {
        matches!(
            self,
            Label::FullName | Label::Titled { .. } | Label::Npc { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classified<'a> {
    pub channel: Channel,
    /// Speaker to display (inner name for guild/alliance/party chat).
    pub speaker: &'a str,
    pub flags: u8,
    pub label: Option<Label<'a>>,
    /// Signed combat number (`-190`) when the text is one.
    pub amount: Option<i64>,
    /// Guild abbreviation that prefixes alliance chat (`[OAK] ...`).
    pub chat_guild: Option<&'a str>,
}

/// Context the classifier needs from the surrounding file.
pub struct Ctx<'c> {
    /// Character currently logged in on this journal, if known.
    pub self_name: Option<&'c str>,
    /// Previous line was `Staff message from X:` – this one is its body.
    pub staff_body: bool,
    pub is_npc: &'c dyn Fn(&str) -> bool,
    pub is_pet: &'c dyn Fn(&str) -> bool,
}

/// A user-defined classification rule (from the settings file).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserRule {
    pub name: String,
    pub enabled: bool,
    /// Only lines from this speaker (case-insensitive). Empty = any speaker.
    pub speaker: String,
    /// Only lines currently classified into one of these. Empty = any channel.
    pub from: ChannelSet,
    /// Regular expression matched against the message text.
    pub pattern: String,
    /// Channel to move matching lines to.
    pub to: Channel,
}

impl Default for UserRule {
    fn default() -> Self {
        UserRule {
            name: "New rule".into(),
            enabled: true,
            speaker: String::new(),
            from: ChannelSet::EMPTY,
            pattern: String::new(),
            to: Channel::System,
        }
    }
}

pub struct CompiledRule {
    speaker: Option<String>,
    from: ChannelSet,
    re: Regex,
    to: Channel,
}

impl CompiledRule {
    pub fn compile(rule: &UserRule) -> Result<CompiledRule, regex::Error> {
        Ok(CompiledRule {
            speaker: Some(rule.speaker.trim().to_lowercase()).filter(|s| !s.is_empty()),
            from: rule.from,
            re: Regex::new(&rule.pattern)?,
            to: rule.to,
        })
    }

    pub fn apply(&self, speaker: &str, text: &str, channel: Channel) -> Option<Channel> {
        if !self.from.is_empty() && !self.from.contains(channel) {
            return None;
        }
        if let Some(s) = &self.speaker {
            if !speaker.eq_ignore_ascii_case(s) && speaker.to_lowercase() != *s {
                return None;
            }
        }
        self.re.is_match(text).then_some(self.to)
    }
}

/// Compile the enabled rules, collecting errors (rule name + message).
pub fn compile_rules(rules: &[UserRule]) -> (Vec<CompiledRule>, Vec<String>) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in rules
        .iter()
        .filter(|r| r.enabled && !r.pattern.trim().is_empty())
    {
        match CompiledRule::compile(r) {
            Ok(c) => ok.push(c),
            Err(e) => errs.push(format!("rule '{}': {e}", r.name)),
        }
    }
    (ok, errs)
}

const CLIENT_PATTERNS: &[&str] = &[
    r"^WorldMap (loading|loaded|markers loaded)",
    r"^Loading WorldMap",
    r"^\.\..*\.(xml|csv|txt) \(\d+\)$",
    r"^(Razor|ClassicUO|Client) (version|v\d)",
];

const WORLD_PATTERNS: &[&str] = &[
    r" has completed the achievement: ",
    r" has registered an antiquity: ",
    r" guild has reached prestige level ",
    r"^The world (will save|is saving)",
    r"^World save complete",
    r"^\[[A-Z]{1,5}\] ",
    r"^An Omni Realm ",
    r"^A contested .*(boss|spawn)",
    r" has fallen to corruption",
    r"^Staff message from ",
    r" has been given the rank of ",
    r"^Our guild now has the opportunity",
    r"^Your guild has summoned",
    r"^Your guild's prestige has",
    r"(?i)^(a|the) .* (event|tournament|invasion) (will begin|has begun|has ended)",
    r"(?i)^server (restart|shutdown|will restart)",
];

const SKILL_PATTERNS: &[&str] = &[
    r"^Your skill in .+ has (increased|decreased)",
    r"(?i)skill ?gain",
    r"(?i)maximum skill cap",
    r"^You must wait a few moments to use another skill",
    r"^Your (strength|dexterity|intelligence) has (increased|decreased|changed)",
    r"^You feel (stronger|more agile|smarter)",
    r"^Your skill .* lock",
];

const COMBAT_PATTERNS: &[&str] = &[
    r"(?i)\battack(s|ing|ed)?\b",
    r"(?i)\bpoison",
    r"(?i)\bparr(y|ied|ies)\b",
    r"(?i)\bfizzle",
    r"(?i)concentration is disturbed",
    r"(?i)not yet recovered from casting",
    r"(?i)already casting",
    r"(?i)\bto cast this spell",
    r"(?i)\b(paralyz|frozen in place|you are frozen|cannot move)",
    r"(?i)\bbandage",
    r"(?i)\byou (heal|cure)\b|\bhas been healed\b|\bbarely help",
    r"(?i)\bresurrect",
    r"(?i)\b(damage|dmg)\b",
    r"(?i)\b(bleed(ing)?|hamstrung|hamstring|disarm(ed)?|stunn?(ed)?|concussion|crippl\w*|mortal(ly)? wound\w*)\b",
    r"(?i)\b(slain|you are dead|you have died)\b",
    r"(?i)\bcritical (hit|strike)",
    r"(?i)\bweapon (special|ability)",
    r"(?i)\bwar mode\b",
    r"(?i)\b(provok|provoc|peacemak|discord)",
    r"(?i)line of sight",
    r"(?i)target (is out of range|cannot be seen)",
    r"(?i)\byou have been (hit|struck|wounded)",
    r"(?i)\blooks? (ill|furious|enraged|frenzied|weakened|very ill)",
    r"(?i)\b(enrage|frenz)",
    r"(?i)\b(criminal|murderer|aggressor|aggressive action)\b",
    r"(?i)\bguards? (have|are) (been )?(called|coming)",
    r"(?i)\bfeel (very ill|a bit nauseous|disoriented|extremely weak|cured)",
    r"(?i)\b(severe|extreme) pain",
];

/// Magery, Necromancy and Chivalry power words → spell name.
const SPELLS: &[(&str, &str)] = &[
    ("Uus Jux", "Clumsy"),
    ("In Mani Ylem", "Create Food"),
    ("Rel Wis", "Feeblemind"),
    ("In Mani", "Heal"),
    ("In Por Ylem", "Magic Arrow"),
    ("In Lor", "Night Sight"),
    ("Flam Sanct", "Reactive Armor"),
    ("Des Mani", "Weaken"),
    ("Ex Uus", "Agility"),
    ("Uus Wis", "Cunning"),
    ("An Nox", "Cure"),
    ("An Mani", "Harm"),
    ("In Jux", "Magic Trap"),
    ("An Jux", "Magic Untrap"),
    ("Uus Sanct", "Protection"),
    ("Uus Mani", "Strength"),
    ("Rel Sanct", "Bless"),
    ("Vas Flam", "Fireball"),
    ("An Por", "Magic Lock"),
    ("In Nox", "Poison"),
    ("Ort Por Ylem", "Telekinesis"),
    ("Rel Por", "Teleport"),
    ("Ex Por", "Unlock"),
    ("In Sanct Ylem", "Wall of Stone"),
    ("Vas An Nox", "Arch Cure"),
    ("Vas Uus Sanct", "Arch Protection"),
    ("Des Sanct", "Curse"),
    ("In Flam Grav", "Fire Field"),
    ("In Vas Mani", "Greater Heal"),
    ("Por Ort Grav", "Lightning"),
    ("Ort Rel", "Mana Drain"),
    ("Kal Ort Por", "Recall"),
    ("In Jux Hur Ylem", "Blade Spirits"),
    ("An Grav", "Dispel Field"),
    ("Kal In Ex", "Incognito"),
    ("In Jux Sanct", "Magic Reflection"),
    ("Por Corp Wis", "Mind Blast"),
    ("An Ex Por", "Paralyze"),
    ("In Nox Grav", "Poison Field"),
    ("Kal Xen", "Summon Creature"),
    ("An Ort", "Dispel"),
    ("Corp Por", "Energy Bolt"),
    ("Vas Ort Flam", "Explosion"),
    ("An Lor Xen", "Invisibility"),
    ("Kal Por Ylem", "Mark"),
    ("Vas Des Sanct", "Mass Curse"),
    ("In Ex Grav", "Paralyze Field"),
    ("Wis Quas", "Reveal"),
    ("Vas Ort Grav", "Chain Lightning"),
    ("In Sanct Grav", "Energy Field"),
    ("Kal Vas Flam", "Flamestrike"),
    ("Vas Rel Por", "Gate Travel"),
    ("Ort Sanct", "Mana Vampire"),
    ("Vas An Ort", "Mass Dispel"),
    ("Flam Kal Des Ylem", "Meteor Swarm"),
    ("Vas Ylem Rel", "Polymorph"),
    ("In Vas Por", "Earthquake"),
    ("Vas Corp Por", "Energy Vortex"),
    ("An Corp", "Resurrection"),
    ("Kal Vas Xen Hur", "Air Elemental"),
    ("Kal Vas Xen Corp", "Summon Daemon"),
    ("Kal Vas Xen Ylem", "Earth Elemental"),
    ("Kal Vas Xen Flam", "Fire Elemental"),
    ("Kal Vas Xen An Flam", "Water Elemental"),
    ("Uus Corp", "Animate Dead"),
    ("In Jux Mani Xen", "Blood Oath"),
    ("In Agle Corp Ylem", "Corpse Skin"),
    ("An Sanct Gra Char", "Curse Weapon"),
    ("Pas Tym An Sanct", "Evil Omen"),
    ("Rel Xen Vas Bal", "Horrific Beast"),
    ("Rel Xen Corp Ort", "Lich Form"),
    ("Wis An Ben", "Mind Rot"),
    ("In Sar", "Pain Spike"),
    ("In Vas Nox", "Poison Strike"),
    ("In Bal Nox", "Strangle"),
    ("Kal Xen Bal", "Summon Familiar"),
    ("Rel Xen An Sanct", "Vampiric Embrace"),
    ("Kal Xen Bal Beh", "Vengeful Spirit"),
    ("Kal Vas An Flam", "Wither"),
    ("Rel Xen Um", "Wraith Form"),
    ("Ort Corp Grav", "Exorcism"),
    ("Expor Flamus", "Cleanse by Fire"),
    ("Obsu Vulni", "Close Wounds"),
    ("Consecrus Arma", "Consecrate Weapon"),
    ("Dispiro Malas", "Dispel Evil"),
    ("Divinum Furis", "Divine Fury"),
    ("Forul Solum", "Enemy of One"),
    ("Augus Luminos", "Holy Light"),
    ("Dium Prostra", "Noble Sacrifice"),
    ("Extermo Vomica", "Remove Curse"),
    ("Sanctum Viatas", "Sacred Journey"),
];

pub struct Classifier {
    client: RegexSet,
    world: RegexSet,
    skill: RegexSet,
    combat: RegexSet,
    /// `Kal Ort Por [Recall]` (ClassicUO's "{power} [{spell}]" spell format).
    spell_fmt: Regex,
    /// `all follow me`, `all kill`, ... (prefix checked separately for pet names).
    pet_cmd: Regex,
    /// Aspect experience gains.
    aspect_xp: Regex,
    spells: FxHashMap<String, &'static str>,
    /// System messages repeat a lot (targeting prompts, crafting results, …);
    /// remember their channel instead of re-running the sets.
    system_cache: RefCell<FxHashMap<Box<str>, Channel>>,
}

const SYSTEM_CACHE_CAP: usize = 8192;

impl Default for Classifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Classifier {
    pub fn new() -> Self {
        Classifier {
            client: RegexSet::new(CLIENT_PATTERNS).expect("client patterns"),
            world: RegexSet::new(WORLD_PATTERNS).expect("world patterns"),
            skill: RegexSet::new(SKILL_PATTERNS).expect("skill patterns"),
            combat: RegexSet::new(COMBAT_PATTERNS).expect("combat patterns"),
            pet_cmd: Regex::new(
                r"(?i)^(.+?) (kill|attack|guard|guard me|follow|follow me|come|stay|stop|patrol|fetch|get|drop|friend|transfer|release|go)$",
            )
            .expect("pet command"),
            aspect_xp: Regex::new(r"(?i)\baspect\b.*\b(xp|exp|experience)\b|\b(xp|exp|experience)\b.*\baspect\b")
                .expect("aspect xp"),
            spell_fmt: Regex::new(r"^(?:[A-Z][a-z]+ ){0,5}[A-Z][a-z]+ \[[A-Z][A-Za-z' ]{1,30}\]$")
                .expect("spell fmt"),
            spells: SPELLS
                .iter()
                .map(|(w, n)| (w.to_ascii_lowercase(), *n))
                .collect(),
            system_cache: RefCell::new(FxHashMap::default()),
        }
    }

    pub fn is_combat_text(&self, text: &str) -> bool {
        self.combat.is_match(text)
    }

    /// Spell name for bare power words (`Kal Ort Por` → `Recall`).
    pub fn spell_name(&self, words: &str) -> Option<&'static str> {
        if words.len() > 24 {
            return None;
        }
        self.spells.get(&words.trim().to_ascii_lowercase()).copied()
    }

    fn system(&self, text: &str, staff_body: bool) -> Channel {
        if staff_body {
            return Channel::World;
        }
        if let Some(&c) = self.system_cache.borrow().get(text) {
            return c;
        }
        let c = self.system_uncached(text);
        if text.len() <= 200 {
            let mut cache = self.system_cache.borrow_mut();
            if cache.len() >= SYSTEM_CACHE_CAP {
                cache.clear();
            }
            cache.insert(text.into(), c);
        }
        c
    }

    fn system_uncached(&self, text: &str) -> Channel {
        if self.world.is_match(text) {
            Channel::World
        } else if self.client.is_match(text) {
            Channel::Client
        } else if self.skill.is_match(text) {
            Channel::Skill
        } else if self.combat.is_match(text) {
            Channel::Combat
        } else {
            Channel::System
        }
    }

    pub fn classify<'a>(&self, name: &'a str, text: &'a str, ctx: &Ctx<'_>) -> Classified<'a> {
        let mut out = Classified {
            channel: Channel::Speech,
            speaker: name,
            flags: 0,
            label: None,
            amount: None,
            chat_guild: None,
        };

        // --- Names the client formats specially -------------------------------
        if let Some(rest) = name.strip_prefix('[') {
            if let Some((chan, inner)) = bracket_channel(rest) {
                out.speaker = inner;
                out.channel = match chan {
                    "Guild" => Channel::Guild,
                    "Alliance" => Channel::Alliance,
                    "Party" => Channel::Party,
                    _ => Channel::System,
                };
                if out.channel == Channel::Alliance {
                    out.chat_guild = leading_tag(text);
                }
                if ctx.self_name == Some(inner) {
                    out.flags |= flags::SELF;
                }
                return out;
            }
            // `[Razor]`, `[UOSteam]`, ...
            out.channel = Channel::Razor;
            return out;
        }

        match name {
            "System" => {
                if self.aspect_xp.is_match(text) && !text.contains(" has completed the achievement")
                {
                    out.channel = Channel::System;
                } else if let Some(n) = combat_number(text) {
                    out.channel = Channel::Combat;
                    out.amount = Some(n);
                    out.flags |= flags::INCOMING;
                } else {
                    out.channel = self.system(text, ctx.staff_body);
                }
                return out;
            }
            "Razor" => {
                out.channel = Channel::Razor;
                return out;
            }
            "You see" | "" => {
                out.channel = Channel::Items;
                return out;
            }
            "Chat" => {
                out.channel = Channel::Speech;
                return out;
            }
            _ => {}
        }

        let is_self = ctx.self_name == Some(name);
        if is_self {
            out.flags |= flags::SELF;
        }

        // --- Aspect experience over yourself -> personal system message --------
        if is_self && self.aspect_xp.is_match(text) {
            out.channel = Channel::System;
            out.flags &= !flags::SELF;
            return out;
        }

        // --- Damage / heal numbers over a mobile ------------------------------
        if let Some(n) = combat_number(text) {
            out.channel = Channel::Combat;
            out.amount = Some(n);
            if is_self || (ctx.is_pet)(name) {
                out.flags |= flags::INCOMING;
            }
            return out;
        }

        // --- Name labels -------------------------------------------------------
        if let Some(label) = label_shape(name, text) {
            out.label = Some(label);
            out.channel = match label {
                // Objects (lower-case names) with bracketed state are item labels:
                // `a reagent crate: [locked down]`.
                Label::GuildTag { .. } if !starts_upper(name) => Channel::Items,
                _ => Channel::Names,
            };
            if out.channel == Channel::Items {
                out.label = None;
            }
            return out;
        }

        // --- Pet commands: `all follow me`, `<pet> kill` ---------------------------
        if let Some(m) = self.pet_cmd.captures(text) {
            let target = m.get(1).map(|g| g.as_str()).unwrap_or("");
            if target.eq_ignore_ascii_case("all") || (ctx.is_pet)(target) {
                out.channel = Channel::Combat;
                return out;
            }
        }

        // --- Spells -------------------------------------------------------------
        if self.spell_fmt.is_match(text) || self.spell_name(text).is_some() {
            out.channel = Channel::Spell;
            return out;
        }

        // --- Emotes -------------------------------------------------------------
        if text.len() >= 2 && text.starts_with('*') && text.ends_with('*') {
            let creature = !starts_upper(name) && !(ctx.is_pet)(name);
            out.channel = if self.combat.is_match(text) || creature {
                Channel::Combat
            } else {
                Channel::Emote
            };
            return out;
        }

        // --- Overhead system notices on yourself: "You have hidden yourself well."
        if is_self && (text.starts_with("You ") || text.starts_with("Your ")) {
            out.channel = self.system(text, false);
            out.flags &= !flags::SELF;
            return out;
        }

        // --- Object messages: `a weathered chest: [secured]` ---------------------
        if !starts_upper(name) && text.starts_with('[') && text.ends_with(']') {
            out.channel = Channel::Items;
            return out;
        }

        out.channel = if (ctx.is_npc)(name) {
            Channel::Npc
        } else {
            Channel::Speech
        };
        out
    }
}

/// `Guild][Oswin Pike]` → ("Guild", "Oswin Pike")
fn bracket_channel(rest: &str) -> Option<(&str, &str)> {
    let close = rest.find("][")?;
    let chan = &rest[..close];
    let inner = rest[close + 2..].strip_suffix(']')?;
    if chan.is_empty() || chan.len() > 16 || !chan.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some((chan, inner))
}

/// `[OAK] crypt is clear` → `OAK`
fn leading_tag(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('[')?;
    let close = rest.find(']')?;
    let tag = &rest[..close];
    (!tag.is_empty() && tag.len() <= 8 && !tag.contains(' ')).then_some(tag)
}

/// `-190`, `+25`, `12` → signed amount. Damage is shown negative by the client.
pub fn combat_number(text: &str) -> Option<i64> {
    let t = text.trim();
    let (sign, digits) = match t.as_bytes().first()? {
        b'-' => (-1, &t[1..]),
        b'+' => (1, &t[1..]),
        _ => (1, t),
    };
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Bare positive numbers are ambiguous (could be someone typing "2"); only accept
    // signed ones.
    if sign == 1 && !t.starts_with('+') {
        return None;
    }
    digits.parse::<i64>().ok().map(|v| v * sign)
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().map(|c| !c.is_lowercase()).unwrap_or(false)
}

/// Recognise the shapes the client uses for single-click name labels.
pub fn label_shape<'a>(name: &str, text: &'a str) -> Option<Label<'a>> {
    if name.is_empty() {
        return None;
    }
    if text == name {
        return Some(Label::FullName);
    }
    // `Lord Fennick Dale`, `Lady Marisol`, `Dread Lord X`
    if let Some(prefix) = text.strip_suffix(name).and_then(|p| p.strip_suffix(' ')) {
        if !prefix.is_empty()
            && prefix.len() <= 24
            && prefix
                .split(' ')
                .all(|w| starts_upper(w) && w.chars().all(char::is_alphabetic))
        {
            return Some(Label::Titled { title: prefix });
        }
    }
    // `Garrick the tanner`, `Hollis the Keeper of the Lighthouse`
    if let Some(rest) = text.strip_prefix(name) {
        if let Some(title) = rest.strip_prefix(" the ") {
            if !title.is_empty() && title.len() <= 48 && !title.ends_with(['.', '!', '?']) {
                return Some(Label::Npc {
                    title: &text[name.len() + 1..],
                });
            }
        }
        // `Name, the Bold`
        if let Some(title) = rest.strip_prefix(", ") {
            if !title.is_empty()
                && title.len() <= 32
                && !title.ends_with(['.', '!', '?'])
                && title.split(' ').count() <= 4
            {
                return Some(Label::Titled { title });
            }
        }
    }
    // `[Steward, OAK]`
    if text.len() >= 3 && text.len() <= 48 && text.starts_with('[') && text.ends_with(']') {
        let inner = &text[1..text.len() - 1];
        if !inner.contains(['[', ']']) {
            return Some(Label::GuildTag { inner });
        }
    }
    // `(bonded)`, `(tame)`, `(summoned)`
    if text.len() >= 3 && text.len() <= 32 && text.starts_with('(') && text.ends_with(')') {
        let inner = &text[1..text.len() - 1];
        if !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_alphabetic() || c == ' ' || c == '-')
        {
            return Some(Label::Status { status: inner });
        }
    }
    // `*released*` on a pet
    if text == "*released*" {
        return Some(Label::Status { status: "released" });
    }
    None
}

/// Could this line be a title line in front of a name label (`Seasoned Angler`)?
pub fn looks_like_title(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.len() <= 40
        && starts_upper(t)
        && t.split(' ').count() <= 5
        && !t.ends_with(['.', '!', '?', ',', ':'])
        && !t.contains("http")
}

#[cfg(test)]
mod tests {
    use super::*;

    // All names, tags and chat below are invented.

    fn ctx_none() -> (impl Fn(&str) -> bool, impl Fn(&str) -> bool) {
        (|n: &str| n == "Garrick", |n: &str| n == "Smudge")
    }

    fn c(name: &str, text: &str) -> (Channel, String, u8) {
        let cl = Classifier::new();
        let (npc, pet) = ctx_none();
        let ctx = Ctx {
            self_name: Some("Aldric Thorne"),
            staff_body: false,
            is_npc: &npc,
            is_pet: &pet,
        };
        let r = cl.classify(name, text, &ctx);
        (r.channel, r.speaker.to_string(), r.flags)
    }

    #[test]
    fn chat_channels() {
        assert_eq!(
            c("[Guild][Oswin Pike]", "east wing clear").0,
            Channel::Guild
        );
        assert_eq!(c("[Guild][Oswin Pike]", "east wing clear").1, "Oswin Pike");
        assert_eq!(
            c("[Alliance][Lysa Quill]", "[OAK] anyone up for the crypt?").0,
            Channel::Alliance
        );
        assert_eq!(c("[Party][Bramblewick]", "heal me").0, Channel::Party);
        assert_eq!(
            c("[Razor]", "Warning: Nightshade amount is now 2!").0,
            Channel::Razor
        );
        assert_eq!(c("Razor", "[world save]").0, Channel::Razor);
    }

    #[test]
    fn system_buckets() {
        assert_eq!(c("System", "WorldMap loading...").0, Channel::Client);
        assert_eq!(c("System", "..Custom_Markers.xml (7)").0, Channel::Client);
        assert_eq!(
            c(
                "System",
                "Corwen Ash has completed the achievement: Angler (Basic)."
            )
            .0,
            Channel::World
        );
        assert_eq!(
            c("System", "The world will save in 30 seconds.").0,
            Channel::World
        );
        assert_eq!(
            c("System", "[EV] A Harvestide event will begin in 5 minutes.").0,
            Channel::World
        );
        assert_eq!(
            c(
                "System",
                "Your skill in Fishing has increased by 0.2.  It is now 64.8."
            )
            .0,
            Channel::Skill
        );
        assert_eq!(c("System", "ItemID skillgain: 2.5%").0, Channel::Skill);
        assert_eq!(
            c("System", "Your Song of Discordance effect ends.").0,
            Channel::Combat
        );
        assert_eq!(c("System", "You feel very ill.").0, Channel::Combat);
        assert_eq!(c("System", "The trade was cancelled.").0, Channel::System);
        assert_eq!(c("System", "-7").0, Channel::Combat);
    }

    #[test]
    fn combat_numbers() {
        assert_eq!(c("a frost troll", "-210").0, Channel::Combat);
        let (ch, _, fl) = c("Smudge", "-9");
        assert_eq!(ch, Channel::Combat);
        assert!(fl & flags::INCOMING != 0);
        let (_, _, fl) = c("a frost troll", "-210");
        assert!(fl & flags::INCOMING == 0);
        assert_eq!(c("a frost troll", "*looks enraged*").0, Channel::Combat);
        assert_eq!(combat_number("-512"), Some(-512));
        assert_eq!(combat_number("+25"), Some(25));
        assert_eq!(combat_number("2"), None);
        assert_eq!(combat_number("-"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(c("Fennick Dale", "[Steward, OAK]").0, Channel::Names);
        assert_eq!(c("Fennick Dale", "Lord Fennick Dale").0, Channel::Names);
        assert_eq!(c("Garrick", "Garrick the tanner").0, Channel::Names);
        assert_eq!(c("Pip", "(bonded)").0, Channel::Names);
        assert_eq!(c("a dire wolf", "a dire wolf").0, Channel::Names);
        assert_eq!(c("Pip", "*released*").0, Channel::Names);
        assert_eq!(
            c("a reagent crate", "[no longer locked down]").0,
            Channel::Items
        );
        assert_eq!(c("a weathered chest", "[secured]").0, Channel::Items);
        assert_eq!(c("You see", "a glowing violet runebook").0, Channel::Items);
        assert_eq!(c("", "a pile of logs").0, Channel::Items);
    }

    #[test]
    fn speech_and_spells() {
        assert_eq!(c("Aldric Thorne", "Kal Ort Por [Recall]").0, Channel::Spell);
        assert_eq!(c("Bramblewick", "In Vas Mani").0, Channel::Spell);
        assert_eq!(c("Lysa Quill", "*waves*").0, Channel::Emote);
        assert_eq!(c("Lysa Quill", "good hunting").0, Channel::Speech);
        assert_eq!(c("Garrick", "Fine hides, cheap!").0, Channel::Npc);
        assert_eq!(c("Aldric Thorne", "All Follow Me").0, Channel::Combat);
        assert_eq!(c("Aldric Thorne", "all kill").0, Channel::Combat);
        assert_eq!(c("Aldric Thorne", "Smudge guard me").0, Channel::Combat);
        assert_eq!(c("Lysa Quill", "please stop").0, Channel::Speech);
        assert_eq!(
            c("System", "You gained 15 aspect experience.").0,
            Channel::System
        );
        assert_eq!(c("Aldric Thorne", "+40 Aspect XP").0, Channel::System);
        assert_eq!(
            c(
                "System",
                "Corwen Ash has completed the achievement: Aspect Mastery (Basic)."
            )
            .0,
            Channel::World
        );
        let (ch, _, fl) = c("Aldric Thorne", "bank");
        assert_eq!(ch, Channel::Speech);
        assert!(fl & flags::SELF != 0);
        assert_eq!(
            c("Aldric Thorne", "You have hidden yourself well.").0,
            Channel::System
        );
    }

    #[test]
    fn user_rules() {
        let rule = UserRule {
            name: "reds".into(),
            speaker: "system".into(),
            pattern: "(?i)reds?".into(),
            to: Channel::Combat,
            ..Default::default()
        };
        let (rules, errs) = compile_rules(&[rule]);
        assert!(errs.is_empty());
        assert_eq!(
            rules[0].apply("System", "4 reds", Channel::System),
            Some(Channel::Combat)
        );
        assert_eq!(
            rules[0].apply("Oswin Pike", "4 reds", Channel::Speech),
            None
        );
    }
}
