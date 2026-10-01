//! Colour themes. A [`Theme`] is the user-editable, serialisable description;
//! [`Palette`] is the resolved form used while painting.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, CornerRadius, Stroke};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uoj_core::Channel;

/// `#rrggbb` colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub const fn hex(v: u32) -> Rgb {
        Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }

    pub fn color(self) -> Color32 {
        Color32::from_rgb(self.0[0], self.0[1], self.0[2])
    }

    pub fn from_color(c: Color32) -> Rgb {
        Rgb([c.r(), c.g(), c.b()])
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Rgb::hex)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("invalid colour '{s}', expected #rrggbb"))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    /// Journal background.
    pub background: Rgb,
    /// Panels, tab bars, menus.
    pub panel: Rgb,
    /// Buttons, text fields, chips.
    pub surface: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    /// Timestamps and secondary text.
    pub dim: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    /// Background behind search matches.
    pub search_match: Rgb,
    /// Row tint for lines that mention one of your characters.
    pub mention: Rgb,
    /// Damage you or your pets took.
    pub damage_taken: Rgb,
    /// Damage on other mobiles.
    pub damage_dealt: Rgb,
    /// Positive combat numbers.
    pub heal: Rgb,
    /// Speaker names when name colouring is off.
    pub name_color: Rgb,
    /// Colours speaker names are hashed into when name colouring is on.
    pub name_palette: Vec<Rgb>,
    /// Text colour per channel, keyed by channel name.
    pub channels: BTreeMap<String, Rgb>,
}

impl Default for Theme {
    fn default() -> Self {
        presets().remove(0)
    }
}

fn channel_map(colors: [(Channel, u32); Channel::COUNT]) -> BTreeMap<String, Rgb> {
    colors
        .iter()
        .map(|(c, v)| (c.label().to_string(), Rgb::hex(*v)))
        .collect()
}

/// Built-in themes. The first one is the default.
pub fn presets() -> Vec<Theme> {
    use Channel::*;
    vec![
        Theme {
            name: "Midnight".into(),
            dark: true,
            background: Rgb::hex(0x0e1117),
            panel: Rgb::hex(0x161b24),
            surface: Rgb::hex(0x222a36),
            border: Rgb::hex(0x2c3442),
            text: Rgb::hex(0xd8dee9),
            dim: Rgb::hex(0x6b7689),
            accent: Rgb::hex(0x7aa2f7),
            selection: Rgb::hex(0x2b3b5c),
            search_match: Rgb::hex(0x6b5310),
            mention: Rgb::hex(0x3a2440),
            damage_taken: Rgb::hex(0xff6b6b),
            damage_dealt: Rgb::hex(0xffb86b),
            heal: Rgb::hex(0x7ee787),
            name_color: Rgb::hex(0xc0caf5),
            name_palette: [
                0x7aa2f7, 0xbb9af7, 0x7dcfff, 0x9ece6a, 0xe0af68, 0xf7768e, 0x73daca, 0xff9e64,
                0xb4f9f8, 0xc3e88d, 0xffc777, 0xfca7ea,
            ]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
            channels: channel_map([
                (Speech, 0xe6e9ef),
                (Npc, 0x9aa5b8),
                (Emote, 0xe0af68),
                (Spell, 0x7aa2f7),
                (Guild, 0x9ece6a),
                (Alliance, 0x73daca),
                (Party, 0xbb9af7),
                (Combat, 0xff7a85),
                (Skill, 0x7dcfff),
                (System, 0xa9b1c6),
                (World, 0x8b93a8),
                (Names, 0xc0caf5),
                (Items, 0xffa96b),
                (Razor, 0x98a0b4),
                (Client, 0x555e70),
            ]),
        },
        Theme {
            name: "Daylight".into(),
            dark: false,
            background: Rgb::hex(0xfbfbfa),
            panel: Rgb::hex(0xeef0f3),
            surface: Rgb::hex(0xe1e5eb),
            border: Rgb::hex(0xc9cfd8),
            text: Rgb::hex(0x1f2328),
            dim: Rgb::hex(0x8a94a3),
            accent: Rgb::hex(0x0969da),
            selection: Rgb::hex(0xcfe2ff),
            search_match: Rgb::hex(0xfff0a0),
            mention: Rgb::hex(0xfde2f3),
            damage_taken: Rgb::hex(0xcf222e),
            damage_dealt: Rgb::hex(0xbc4c00),
            heal: Rgb::hex(0x1a7f37),
            name_color: Rgb::hex(0x24292f),
            name_palette: [
                0x0550ae, 0x8250df, 0x1a7f37, 0xbc4c00, 0xcf222e, 0x0a7a8a, 0x953800, 0x6639ba,
                0x116329, 0xa40e26,
            ]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
            channels: channel_map([
                (Speech, 0x1f2328),
                (Npc, 0x6e7781),
                (Emote, 0x9a6700),
                (Spell, 0x0550ae),
                (Guild, 0x1a7f37),
                (Alliance, 0x0a7a8a),
                (Party, 0x8250df),
                (Combat, 0xcf222e),
                (Skill, 0x0969da),
                (System, 0x57606a),
                (World, 0x6e7781),
                (Names, 0x24292f),
                (Items, 0xbc4c00),
                (Razor, 0x6e7781),
                (Client, 0xafb8c1),
            ]),
        },
        Theme {
            name: "Britannia".into(),
            dark: true,
            background: Rgb::hex(0x1b1510),
            panel: Rgb::hex(0x251d16),
            surface: Rgb::hex(0x36291d),
            border: Rgb::hex(0x4a3826),
            text: Rgb::hex(0xeadbc0),
            dim: Rgb::hex(0x8c7a60),
            accent: Rgb::hex(0xe0b04a),
            selection: Rgb::hex(0x4a3a22),
            search_match: Rgb::hex(0x7a5a10),
            mention: Rgb::hex(0x4a2424),
            damage_taken: Rgb::hex(0xff6a4d),
            damage_dealt: Rgb::hex(0xf0b45a),
            heal: Rgb::hex(0x9fd36a),
            name_color: Rgb::hex(0xf2d9a2),
            name_palette: [
                0xe0b04a, 0xd98a5f, 0x9fd36a, 0x7fc4c4, 0xc49ae0, 0xe07a7a, 0xb8c46a, 0x8fb0e0,
                0xe0c47a, 0xc4a07a,
            ]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
            channels: channel_map([
                (Speech, 0xf2e6cf),
                (Npc, 0xb3a180),
                (Emote, 0xf0b45a),
                (Spell, 0x8fb0e0),
                (Guild, 0x9fd36a),
                (Alliance, 0x7fc4c4),
                (Party, 0xc49ae0),
                (Combat, 0xff7a5c),
                (Skill, 0x8fc0d8),
                (System, 0xc8b898),
                (World, 0x9c8a6c),
                (Names, 0xf2d9a2),
                (Items, 0xe8a86a),
                (Razor, 0xa89878),
                (Client, 0x5e5040),
            ]),
        },
        Theme {
            name: "Classic UO".into(),
            dark: true,
            background: Rgb::hex(0x000000),
            panel: Rgb::hex(0x141414),
            surface: Rgb::hex(0x262626),
            border: Rgb::hex(0x3a3a3a),
            text: Rgb::hex(0xe8e8e8),
            dim: Rgb::hex(0x7a7a7a),
            accent: Rgb::hex(0x39b0ff),
            selection: Rgb::hex(0x203a5a),
            search_match: Rgb::hex(0x665500),
            mention: Rgb::hex(0x3d1f3d),
            damage_taken: Rgb::hex(0xff3030),
            damage_dealt: Rgb::hex(0xffcc00),
            heal: Rgb::hex(0x40ff40),
            name_color: Rgb::hex(0xa0c8ff),
            name_palette: [
                0x39b0ff, 0xffcc00, 0x40ff40, 0xff8040, 0xff60ff, 0x40ffff, 0xffff80, 0x8080ff,
            ]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
            channels: channel_map([
                (Speech, 0x5ab4ff),
                (Npc, 0x8fb8d8),
                (Emote, 0xff9933),
                (Spell, 0x6699ff),
                (Guild, 0x44dd44),
                (Alliance, 0x57d7a0),
                (Party, 0x29c7ff),
                (Combat, 0xff4040),
                (Skill, 0x33ccff),
                (System, 0xe8e8e8),
                (World, 0xb0b0b0),
                (Names, 0xffff99),
                (Items, 0xd0d0d0),
                (Razor, 0x80ff80),
                (Client, 0x606060),
            ]),
        },
        Theme {
            name: "Nord".into(),
            dark: true,
            background: Rgb::hex(0x2e3440),
            panel: Rgb::hex(0x3b4252),
            surface: Rgb::hex(0x434c5e),
            border: Rgb::hex(0x4c566a),
            text: Rgb::hex(0xeceff4),
            dim: Rgb::hex(0x8690a5),
            accent: Rgb::hex(0x88c0d0),
            selection: Rgb::hex(0x4c566a),
            search_match: Rgb::hex(0x7b6a3a),
            mention: Rgb::hex(0x5a4256),
            damage_taken: Rgb::hex(0xbf616a),
            damage_dealt: Rgb::hex(0xd08770),
            heal: Rgb::hex(0xa3be8c),
            name_color: Rgb::hex(0xe5e9f0),
            name_palette: [
                0x88c0d0, 0x81a1c1, 0xa3be8c, 0xebcb8b, 0xd08770, 0xb48ead, 0x8fbcbb, 0xbf616a,
            ]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
            channels: channel_map([
                (Speech, 0xeceff4),
                (Npc, 0xa3abbd),
                (Emote, 0xebcb8b),
                (Spell, 0x81a1c1),
                (Guild, 0xa3be8c),
                (Alliance, 0x8fbcbb),
                (Party, 0xb48ead),
                (Combat, 0xbf616a),
                (Skill, 0x88c0d0),
                (System, 0xd8dee9),
                (World, 0x9aa3b5),
                (Names, 0xe5e9f0),
                (Items, 0xd08770),
                (Razor, 0xa3abbd),
                (Client, 0x616e88),
            ]),
        },
        Theme {
            name: "High Contrast".into(),
            dark: true,
            background: Rgb::hex(0x000000),
            panel: Rgb::hex(0x0a0a0a),
            surface: Rgb::hex(0x1e1e1e),
            border: Rgb::hex(0xffffff),
            text: Rgb::hex(0xffffff),
            dim: Rgb::hex(0xb0b0b0),
            accent: Rgb::hex(0xffff00),
            selection: Rgb::hex(0x0037a0),
            search_match: Rgb::hex(0x8a6d00),
            mention: Rgb::hex(0x5a005a),
            damage_taken: Rgb::hex(0xff4444),
            damage_dealt: Rgb::hex(0xffd000),
            heal: Rgb::hex(0x00ff66),
            name_color: Rgb::hex(0xffffff),
            name_palette: [0x00ffff, 0xffff00, 0x00ff66, 0xff9900, 0xff66ff, 0x66b3ff]
                .into_iter()
                .map(Rgb::hex)
                .collect(),
            channels: channel_map([
                (Speech, 0xffffff),
                (Npc, 0xd0d0d0),
                (Emote, 0xffcc00),
                (Spell, 0x66b3ff),
                (Guild, 0x00ff66),
                (Alliance, 0x00ffff),
                (Party, 0xff66ff),
                (Combat, 0xff4444),
                (Skill, 0x66d9ff),
                (System, 0xf0f0f0),
                (World, 0xc8c8c8),
                (Names, 0xffff99),
                (Items, 0xffaa55),
                (Razor, 0xc8c8c8),
                (Client, 0x909090),
            ]),
        },
    ]
}

/// Resolved colours for fast painting.
#[derive(Clone, Debug)]
pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub search_match: Color32,
    pub mention: Color32,
    pub damage_taken: Color32,
    pub damage_dealt: Color32,
    pub heal: Color32,
    pub name: Color32,
    pub name_palette: Vec<Color32>,
    pub channels: [Color32; Channel::COUNT],
}

impl Palette {
    pub fn from_theme(t: &Theme) -> Palette {
        let fallback = Theme::default();
        let mut channels = [t.text.color(); Channel::COUNT];
        for c in Channel::ALL {
            let col = t
                .channels
                .get(c.label())
                .or_else(|| fallback.channels.get(c.label()))
                .copied()
                .unwrap_or(t.text);
            channels[c.index()] = col.color();
        }
        let mut name_palette: Vec<Color32> = t.name_palette.iter().map(|c| c.color()).collect();
        if name_palette.is_empty() {
            name_palette.push(t.name_color.color());
        }
        Palette {
            dark: t.dark,
            background: t.background.color(),
            panel: t.panel.color(),
            surface: t.surface.color(),
            border: t.border.color(),
            text: t.text.color(),
            dim: t.dim.color(),
            accent: t.accent.color(),
            selection: t.selection.color(),
            search_match: t.search_match.color(),
            mention: t.mention.color(),
            damage_taken: t.damage_taken.color(),
            damage_dealt: t.damage_dealt.color(),
            heal: t.heal.color(),
            name: t.name_color.color(),
            name_palette,
            channels,
        }
    }

    #[inline]
    pub fn channel(&self, c: Channel) -> Color32 {
        self.channels[c.index()]
    }

    /// Stable colour for a name (FNV-1a hash into the palette).
    pub fn name_color(&self, name: &str) -> Color32 {
        let mut h: u32 = 0x811c9dc5;
        for b in name.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        self.name_palette[(h as usize) % self.name_palette.len()]
    }

    /// egui visuals matching the theme.
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let radius = CornerRadius::same(4);
        v.override_text_color = None;
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.extreme_bg_color = self.background;
        v.faint_bg_color = self.surface.gamma_multiply(0.5);
        v.code_bg_color = self.surface;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.selection;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(6);
        v.weak_text_color = Some(self.dim);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = self.surface;
        w.inactive.weak_bg_fill = self.surface;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border.gamma_multiply(0.6));
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;

        let hover = blend(self.surface, self.accent, 0.18);
        w.hovered.bg_fill = hover;
        w.hovered.weak_bg_fill = hover;
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.7));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.hovered.corner_radius = radius;

        let active = blend(self.surface, self.accent, 0.35);
        w.active.bg_fill = active;
        w.active.weak_bg_fill = active;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(2.0, self.text);
        w.active.corner_radius = radius;

        w.open.bg_fill = hover;
        w.open.weak_bg_fill = hover;
        w.open.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.7));
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        w.open.corner_radius = radius;
        v
    }
}

/// Linear blend of two colours (`t` = 0 → a, 1 → b).
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_cover_all_channels() {
        for t in presets() {
            for c in Channel::ALL {
                assert!(
                    t.channels.contains_key(c.label()),
                    "{} lacks {}",
                    t.name,
                    c.label()
                );
            }
        }
    }

    #[test]
    fn rgb_roundtrip() {
        let t = Theme::default();
        let s = toml::to_string(&t).unwrap();
        let back: Theme = toml::from_str(&s).unwrap();
        assert_eq!(t, back);
        assert_eq!(Rgb::parse("#7aa2f7"), Some(Rgb::hex(0x7aa2f7)));
    }
}
