//! Settings file (`uoc-journal.toml`) and pane definitions.
//!
//! Portable mode: if `uoc-journal.toml` sits next to the executable it is used
//! (and written) there; otherwise the per-user config directory is used.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uoj_core::classify::UserRule;
use uoj_core::{Channel, ChannelSet};

use crate::theme::{Rgb, Theme};

pub const SETTINGS_FILE: &str = "uoc-journal.toml";
pub const LAYOUT_FILE: &str = "uoc-journal-layout.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PaneKind {
    #[default]
    Log,
    People,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeFormat {
    #[default]
    Time,
    DateTime,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneConfig {
    pub id: u64,
    pub title: String,
    pub kind: PaneKind,
    pub channels: ChannelSet,
    /// Saved filter expression, combined with the live search box.
    pub filter: String,
    /// Character names to show; empty = all.
    pub characters: Vec<String>,
    pub show_dups: bool,
    /// Show the row of channel toggle chips above the log.
    pub chip_bar: bool,
    /// Show damage totals above the log.
    pub combat_summary: bool,
}

impl Default for PaneConfig {
    fn default() -> Self {
        PaneConfig {
            id: 0,
            title: "Journal".into(),
            kind: PaneKind::Log,
            channels: ChannelSet::ALL,
            filter: String::new(),
            characters: Vec::new(),
            show_dups: false,
            chip_bar: false,
            combat_summary: false,
        }
    }
}

/// Ready-made panes offered in the "New pane" menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    All,
    Chat,
    GuildAlliance,
    Guild,
    Alliance,
    Party,
    Combat,
    Names,
    People,
    Items,
    System,
    World,
    Skills,
    Mentions,
    Npc,
    Custom,
}

impl Template {
    pub const MENU: [Template; 16] = [
        Template::All,
        Template::Chat,
        Template::GuildAlliance,
        Template::Guild,
        Template::Alliance,
        Template::Party,
        Template::Combat,
        Template::People,
        Template::Names,
        Template::Items,
        Template::System,
        Template::Skills,
        Template::World,
        Template::Mentions,
        Template::Npc,
        Template::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Template::All => "All messages",
            Template::Chat => "All chat",
            Template::GuildAlliance => "Guild / Alliance / Party",
            Template::Guild => "Guild",
            Template::Alliance => "Alliance",
            Template::Party => "Party",
            Template::Combat => "Combat",
            Template::Names => "Name labels",
            Template::People => "People (names seen)",
            Template::Items => "Items",
            Template::System => "System",
            Template::World => "World news",
            Template::Skills => "Skills",
            Template::Mentions => "Mentions of you",
            Template::Npc => "NPC speech",
            Template::Custom => "Custom (everything, edit filter)",
        }
    }

    pub fn make(self, id: u64) -> PaneConfig {
        use Channel::*;
        let base = PaneConfig {
            id,
            ..Default::default()
        };
        let with = |title: &str, chans: &[Channel]| PaneConfig {
            title: title.into(),
            channels: ChannelSet::of(chans),
            ..base.clone()
        };
        match self {
            Template::All => {
                let mut c = ChannelSet::ALL;
                c.remove(Client);
                PaneConfig {
                    title: "All".into(),
                    channels: c,
                    chip_bar: true,
                    ..base
                }
            }
            Template::Chat => with("Chat", &[Speech, Emote, Guild, Alliance, Party]),
            Template::GuildAlliance => with("Guild / Ally / Party", &[Guild, Alliance, Party]),
            Template::Guild => with("Guild", &[Guild]),
            Template::Alliance => with("Alliance", &[Alliance]),
            Template::Party => with("Party", &[Party]),
            Template::Combat => PaneConfig {
                combat_summary: true,
                ..with("Combat", &[Combat])
            },
            Template::Names => with("Name labels", &[Names]),
            Template::People => PaneConfig {
                title: "People".into(),
                kind: PaneKind::People,
                ..base
            },
            Template::Items => with("Items", &[Items]),
            Template::System => with("System", &[System, Skill, Razor]),
            Template::World => with("World", &[World]),
            Template::Skills => with("Skills", &[Skill]),
            Template::Mentions => PaneConfig {
                title: "Mentions".into(),
                filter: "is:mention".into(),
                ..with("Mentions", &[Speech, Emote, Guild, Alliance, Party, Npc])
            },
            Template::Npc => with("NPCs", &[Npc]),
            Template::Custom => PaneConfig {
                title: "Custom".into(),
                chip_bar: true,
                ..base
            },
        }
    }
}

/// Colour lines or words that match a pattern; optionally flash the taskbar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HighlightRule {
    pub enabled: bool,
    /// Plain words separated by `|`, or a regular expression when `regex` is set.
    pub pattern: String,
    pub regex: bool,
    pub color: Rgb,
    /// Tint the whole line instead of just the matched words.
    pub whole_line: bool,
    /// Flash the taskbar button when a new line matches while the window is unfocused.
    pub alert: bool,
    /// Only in these channels (empty = all).
    pub channels: ChannelSet,
}

impl Default for HighlightRule {
    fn default() -> Self {
        HighlightRule {
            enabled: true,
            pattern: String::new(),
            regex: false,
            color: Rgb::hex(0xffd166),
            whole_line: false,
            alert: false,
            channels: ChannelSet::EMPTY,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub journal_folder: Option<PathBuf>,
    pub history_hours: u32,
    pub max_history_files: usize,
    pub theme: String,
    pub custom_themes: Vec<Theme>,
    pub font_size: f32,
    pub monospace: bool,
    /// Optional .ttf/.otf used for all text.
    pub font_path: Option<PathBuf>,
    pub time_format: TimeFormat,
    pub show_badges: bool,
    pub color_names: bool,
    /// Prefix lines with the character name when more than one client is logged.
    pub character_tags: bool,
    pub always_on_top: bool,
    pub highlights: Vec<HighlightRule>,
    pub rules: Vec<UserRule>,
    pub panes: Vec<PaneConfig>,
    pub next_pane_id: u64,
    /// Set after the first-run folder prompt was shown.
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            journal_folder: None,
            history_hours: 12,
            max_history_files: 40,
            theme: "Midnight".into(),
            custom_themes: Vec::new(),
            font_size: 14.0,
            monospace: false,
            font_path: None,
            time_format: TimeFormat::Time,
            show_badges: false,
            color_names: true,
            character_tags: true,
            always_on_top: false,
            highlights: vec![
                HighlightRule {
                    pattern: r"\b\d*\s*(reds?|pks?|gankers?|murderers?)\b".into(),
                    regex: true,
                    color: Rgb::hex(0xff5c5c),
                    channels: ChannelSet::of(&[Channel::Alliance, Channel::Guild, Channel::Party]),
                    ..Default::default()
                },
                HighlightRule {
                    pattern: "staff message".into(),
                    color: Rgb::hex(0xffd166),
                    whole_line: true,
                    ..Default::default()
                },
            ],
            rules: Vec::new(),
            panes: Vec::new(),
            next_pane_id: 1,
            onboarded: false,
        }
    }
}

impl Settings {
    pub fn all_themes(&self) -> Vec<Theme> {
        let mut v = crate::theme::presets();
        for t in &self.custom_themes {
            v.retain(|p| p.name != t.name);
            v.push(t.clone());
        }
        v
    }

    pub fn current_theme(&self) -> Theme {
        self.all_themes()
            .into_iter()
            .find(|t| t.name == self.theme)
            .unwrap_or_default()
    }

    pub fn alloc_pane_id(&mut self) -> u64 {
        let used = self.panes.iter().map(|p| p.id).max().unwrap_or(0);
        self.next_pane_id = self.next_pane_id.max(used + 1);
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }
}

/// Where settings live.
pub fn config_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join(SETTINGS_FILE).is_file() || dir.join("portable.txt").is_file() {
                return dir.to_path_buf();
            }
        }
    }
    directories::ProjectDirs::from("", "", "UOC Journal")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn load(dir: &Path) -> (Settings, Option<String>) {
    let path = dir.join(SETTINGS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str::<Settings>(&s) {
            Ok(settings) => (settings, None),
            Err(e) => {
                // Keep the broken file around instead of overwriting the user's edits.
                let _ = std::fs::copy(&path, dir.join(format!("{SETTINGS_FILE}.broken")));
                (
                    Settings::default(),
                    Some(format!("Could not read {}: {e}", path.display())),
                )
            }
        },
        Err(_) => (Settings::default(), None),
    }
}

pub fn save(dir: &Path, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = toml::to_string_pretty(settings).map_err(|e| e.to_string())?;
    write_atomic(&dir.join(SETTINGS_FILE), text.as_bytes())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip() {
        let mut s = Settings {
            panes: Template::MENU
                .iter()
                .enumerate()
                .map(|(i, t)| t.make(i as u64 + 1))
                .collect(),
            ..Default::default()
        };
        s.rules.push(UserRule {
            pattern: "x".into(),
            ..Default::default()
        });
        s.custom_themes.push(Theme {
            name: "Mine".into(),
            ..Default::default()
        });
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_settings_fill_defaults() {
        let s: Settings = toml::from_str("font_size = 16.0\n").unwrap();
        assert_eq!(s.font_size, 16.0);
        assert_eq!(s.history_hours, 12);
    }
}
