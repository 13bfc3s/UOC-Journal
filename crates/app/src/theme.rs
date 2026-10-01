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
    /// Optional row background per channel, keyed by channel name.
    pub channel_backgrounds: BTreeMap<String, Rgb>,
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
    let mut v = handmade();
    v.extend(GENERATED.iter().map(build));
    // Row backgrounds are on by default: a light tint of each channel's colour.
    for t in &mut v {
        for c in Channel::ALL {
            let fg = t.channels.get(c.label()).copied().unwrap_or(t.text);
            let tint = default_background(t, fg);
            t.channel_backgrounds
                .entry(c.label().to_string())
                .or_insert(tint);
        }
    }
    v
}

/// The tint used when a channel's background is switched on.
pub fn default_background(t: &Theme, fg: Rgb) -> Rgb {
    let strength = if t.dark { 0.11 } else { 0.09 };
    Rgb::from_color(blend(t.background.color(), fg.color(), strength))
}

fn handmade() -> Vec<Theme> {
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
            channel_backgrounds: BTreeMap::new(),
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
            channel_backgrounds: BTreeMap::new(),
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
            channel_backgrounds: BTreeMap::new(),
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
            channel_backgrounds: BTreeMap::new(),
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
            channel_backgrounds: BTreeMap::new(),
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
            channel_backgrounds: BTreeMap::new(),
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
    pub channel_bg: [Option<Color32>; Channel::COUNT],
    /// Per-character chip overrides: name → (label, colour).
    pub chips: rustc_hash::FxHashMap<String, (String, Color32)>,
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
        let mut channel_bg = [None; Channel::COUNT];
        for c in Channel::ALL {
            channel_bg[c.index()] = t.channel_backgrounds.get(c.label()).map(|c| c.color());
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
            channel_bg,
            chips: Default::default(),
        }
    }

    /// Row background for a channel, if the theme sets one.
    #[inline]
    pub fn channel_bg(&self, c: Channel) -> Option<Color32> {
        self.channel_bg[c.index()]
    }

    /// Label and colour of a character's chip (custom or derived).
    pub fn chip(&self, character: &str) -> (String, Color32) {
        match self.chips.get(character) {
            Some((label, color)) => (label.clone(), *color),
            None => (
                crate::logview::initials(character),
                self.name_color(character),
            ),
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

/// Compact description of a theme: base colours plus eight accents.
struct Spec {
    name: &'static str,
    dark: bool,
    /// background, panel, surface, border, text, dim, accent
    base: [u32; 7],
    /// red, orange, yellow, green, cyan, blue, purple, pink
    acc: [u32; 8],
}

fn mix(a: u32, b: u32, t: f32) -> Rgb {
    Rgb::from_color(blend(Rgb::hex(a).color(), Rgb::hex(b).color(), t))
}

/// Turn a [`Spec`] into a full theme with consistent channel assignments.
fn build(s: &Spec) -> Theme {
    use Channel::*;
    let [bg, panel, surface, border, text, dim, accent] = s.base;
    let [red, orange, yellow, green, cyan, blue, purple, pink] = s.acc;
    let h = Rgb::hex;
    let channels: BTreeMap<String, Rgb> = [
        (Speech, h(text)),
        (Npc, mix(text, dim, 0.45)),
        (Emote, h(yellow)),
        (Spell, h(blue)),
        (Guild, h(green)),
        (Alliance, h(cyan)),
        (Party, h(pink)),
        (Combat, h(red)),
        (Skill, h(purple)),
        (System, mix(text, dim, 0.3)),
        (World, h(dim)),
        (Names, mix(text, blue, 0.25)),
        (Items, h(orange)),
        (Razor, mix(dim, green, 0.35)),
        (Client, mix(dim, bg, 0.4)),
    ]
    .iter()
    .map(|(c, col)| (c.label().to_string(), *col))
    .collect();
    Theme {
        name: s.name.into(),
        dark: s.dark,
        background: h(bg),
        panel: h(panel),
        surface: h(surface),
        border: h(border),
        text: h(text),
        dim: h(dim),
        accent: h(accent),
        selection: mix(bg, accent, 0.28),
        search_match: mix(bg, yellow, 0.38),
        mention: mix(bg, pink, 0.2),
        damage_taken: h(red),
        damage_dealt: h(orange),
        heal: h(green),
        name_color: mix(text, blue, 0.25),
        name_palette: [blue, purple, cyan, green, yellow, pink, orange, accent, red]
            .into_iter()
            .map(Rgb::hex)
            .collect(),
        channels,
        channel_backgrounds: BTreeMap::new(),
    }
}

#[rustfmt::skip]
const GENERATED: &[Spec] = &[
    // Popular editor palettes (dark)
    Spec { name: "Dracula", dark: true, base: [0x282a36, 0x21222c, 0x44475a, 0x44475a, 0xf8f8f2, 0x6272a4, 0xbd93f9], acc: [0xff5555, 0xffb86c, 0xf1fa8c, 0x50fa7b, 0x8be9fd, 0x82aaff, 0xbd93f9, 0xff79c6] },
    Spec { name: "Solarized Dark", dark: true, base: [0x002b36, 0x073642, 0x0b4250, 0x0f4a59, 0x93a1a1, 0x586e75, 0x268bd2], acc: [0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682] },
    Spec { name: "Gruvbox Dark", dark: true, base: [0x282828, 0x1d2021, 0x3c3836, 0x504945, 0xebdbb2, 0x928374, 0xfabd2f], acc: [0xfb4934, 0xfe8019, 0xfabd2f, 0xb8bb26, 0x8ec07c, 0x83a598, 0xd3869b, 0xf28e9b] },
    Spec { name: "Monokai", dark: true, base: [0x272822, 0x1e1f1c, 0x3e3d32, 0x49483e, 0xf8f8f2, 0x75715e, 0xa6e22e], acc: [0xf92672, 0xfd971f, 0xe6db74, 0xa6e22e, 0x66d9ef, 0x66a3ef, 0xae81ff, 0xff6188] },
    Spec { name: "One Dark", dark: true, base: [0x282c34, 0x21252b, 0x2c313a, 0x3e4451, 0xabb2bf, 0x5c6370, 0x61afef], acc: [0xe06c75, 0xd19a66, 0xe5c07b, 0x98c379, 0x56b6c2, 0x61afef, 0xc678dd, 0xe06c9f] },
    Spec { name: "Tokyo Night", dark: true, base: [0x1a1b26, 0x16161e, 0x292e42, 0x2f334d, 0xc0caf5, 0x565f89, 0x7aa2f7], acc: [0xf7768e, 0xff9e64, 0xe0af68, 0x9ece6a, 0x7dcfff, 0x7aa2f7, 0xbb9af7, 0xff7a93] },
    Spec { name: "Catppuccin Mocha", dark: true, base: [0x1e1e2e, 0x181825, 0x313244, 0x45475a, 0xcdd6f4, 0x7f849c, 0xcba6f7], acc: [0xf38ba8, 0xfab387, 0xf9e2af, 0xa6e3a1, 0x94e2d5, 0x89b4fa, 0xcba6f7, 0xf5c2e7] },
    Spec { name: "Everforest Dark", dark: true, base: [0x2d353b, 0x272e33, 0x3d484d, 0x475258, 0xd3c6aa, 0x859289, 0xa7c080], acc: [0xe67e80, 0xe69875, 0xdbbc7f, 0xa7c080, 0x83c092, 0x7fbbb3, 0xd699b6, 0xe69ab6] },
    Spec { name: "Rosé Pine", dark: true, base: [0x191724, 0x1f1d2e, 0x26233a, 0x403d52, 0xe0def4, 0x6e6a86, 0xc4a7e7], acc: [0xeb6f92, 0xea9a97, 0xf6c177, 0x8fbf9f, 0x9ccfd8, 0x3e8fb0, 0xc4a7e7, 0xebbcba] },
    Spec { name: "Kanagawa", dark: true, base: [0x1f1f28, 0x16161d, 0x2a2a37, 0x363646, 0xdcd7ba, 0x727169, 0x7e9cd8], acc: [0xe46876, 0xffa066, 0xe6c384, 0x98bb6c, 0x7aa89f, 0x7e9cd8, 0x957fb8, 0xd27e99] },
    Spec { name: "Ayu Mirage", dark: true, base: [0x1f2430, 0x191e2a, 0x2a3140, 0x33415e, 0xcbccc6, 0x707a8c, 0xffcc66], acc: [0xf28779, 0xffa759, 0xffd580, 0xbae67e, 0x95e6cb, 0x73d0ff, 0xd4bfff, 0xff8f9f] },
    Spec { name: "Material Ocean", dark: true, base: [0x0f111a, 0x090b10, 0x1f2233, 0x292d3e, 0xa6accd, 0x4b526d, 0x84ffff], acc: [0xf07178, 0xf78c6c, 0xffcb6b, 0xc3e88d, 0x89ddff, 0x82aaff, 0xc792ea, 0xff5370] },
    Spec { name: "Night Owl", dark: true, base: [0x011627, 0x01111d, 0x0b2942, 0x1d3b53, 0xd6deeb, 0x637777, 0x82aaff], acc: [0xef5350, 0xf78c6c, 0xffcb8b, 0xaddb67, 0x7fdbca, 0x82aaff, 0xc792ea, 0xff5874] },
    Spec { name: "Cobalt", dark: true, base: [0x193549, 0x15232d, 0x1f4662, 0x0d3a58, 0xffffff, 0x8a9ba8, 0xffc600], acc: [0xff628c, 0xff9d00, 0xffc600, 0x3ad900, 0x80fcff, 0x0088ff, 0xfb94ff, 0xff7ab2] },
    Spec { name: "Synthwave", dark: true, base: [0x262335, 0x241b2f, 0x34294f, 0x495495, 0xf4eee4, 0x848bbd, 0xff7edb], acc: [0xfe4450, 0xf97e72, 0xfede5d, 0x72f1b8, 0x36f9f6, 0x2ee2fa, 0xb893ce, 0xff7edb] },
    Spec { name: "Palenight", dark: true, base: [0x292d3e, 0x202331, 0x34324a, 0x444267, 0xa6accd, 0x676e95, 0xc792ea], acc: [0xf07178, 0xf78c6c, 0xffcb6b, 0xc3e88d, 0x89ddff, 0x82aaff, 0xc792ea, 0xff5370] },
    Spec { name: "Zenburn", dark: true, base: [0x3f3f3f, 0x383838, 0x4f4f4f, 0x5f5f5f, 0xdcdccc, 0x9f9f8f, 0xf0dfaf], acc: [0xcc9393, 0xdfaf8f, 0xf0dfaf, 0x9fc59f, 0x93e0e3, 0x8cd0d3, 0xdc8cc3, 0xec93d3] },
    Spec { name: "Oceanic Next", dark: true, base: [0x1b2b34, 0x162229, 0x343d46, 0x4f5b66, 0xd8dee9, 0x65737e, 0x6699cc], acc: [0xec5f67, 0xf99157, 0xfac863, 0x99c794, 0x5fb3b3, 0x6699cc, 0xc594c5, 0xe48fa8] },
    Spec { name: "Moonfly", dark: true, base: [0x080808, 0x121212, 0x1c1c1c, 0x323437, 0xbdbdbd, 0x808080, 0x80a0ff], acc: [0xff5454, 0xde935f, 0xe3c78a, 0x8cc85f, 0x79dac8, 0x80a0ff, 0xae81ff, 0xff5189] },
    Spec { name: "GitHub Dark", dark: true, base: [0x0d1117, 0x161b22, 0x21262d, 0x30363d, 0xc9d1d9, 0x8b949e, 0x58a6ff], acc: [0xff7b72, 0xffa657, 0xd29922, 0x7ee787, 0x79c0ff, 0x58a6ff, 0xd2a8ff, 0xff9bce] },
    Spec { name: "Horizon", dark: true, base: [0x1c1e26, 0x16161c, 0x2e303e, 0x3b3e52, 0xd5d8da, 0x6c6f93, 0xe95678], acc: [0xe95678, 0xfab795, 0xfac29a, 0x29d398, 0x59e1e3, 0x26bbd9, 0xb877db, 0xf075b5] },
    Spec { name: "Iceberg", dark: true, base: [0x161821, 0x0f1117, 0x1e2132, 0x2a3158, 0xc6c8d1, 0x6b7089, 0x84a0c6], acc: [0xe27878, 0xe2a478, 0xe9b189, 0xb4be82, 0x89b8c2, 0x84a0c6, 0xa093c7, 0xd295b0] },
    // Britannia-flavoured
    Spec { name: "Moonglow", dark: true, base: [0x140f22, 0x1b1430, 0x2a2047, 0x3a2d60, 0xe6ddff, 0x7d6fa8, 0xb48cff], acc: [0xff6b8b, 0xffa26b, 0xffe08a, 0x8fe3a8, 0x7fd8ff, 0x8fa8ff, 0xc79bff, 0xff8ad8] },
    Spec { name: "Blood Moon", dark: true, base: [0x160809, 0x1f0b0d, 0x341417, 0x4a1c20, 0xf0d6d6, 0x8f6b6b, 0xe0453f], acc: [0xff4d4d, 0xff8c42, 0xffc46b, 0x9ccf6b, 0x7fc8c8, 0x8fa3e0, 0xc78ad6, 0xff7aa8] },
    Spec { name: "Deep Sea", dark: true, base: [0x06131c, 0x0a1b27, 0x102a3a, 0x17394d, 0xcfe8f2, 0x5f8597, 0x2ec4d6], acc: [0xff6f69, 0xffab5e, 0xffe28a, 0x5fd3a0, 0x4fe0e6, 0x5aa9ff, 0xa593ff, 0xff8fc7] },
    Spec { name: "Emerald Isle", dark: true, base: [0x0c1a12, 0x102218, 0x18321f, 0x21452c, 0xdff2e3, 0x6e9179, 0x3ddc84], acc: [0xff6b6b, 0xffaf5f, 0xf2e27a, 0x5ee08f, 0x6fe0d0, 0x78b4ff, 0xc39bff, 0xff92c2] },
    Spec { name: "Frost", dark: true, base: [0x0e1621, 0x132031, 0x1c2d44, 0x263c5a, 0xe8f3ff, 0x7a93b3, 0x9fd8ff], acc: [0xff7a90, 0xffb08a, 0xffe4a3, 0x9fe8c2, 0xa8f0ff, 0x8fc4ff, 0xc3b2ff, 0xffb3d9] },
    Spec { name: "Shadowlord", dark: true, base: [0x0a0a0c, 0x111114, 0x1c1b22, 0x2a2833, 0xd4d0dc, 0x6c6878, 0x9a6bff], acc: [0xe04f5f, 0xe8884a, 0xd8c06a, 0x6fbf73, 0x5fb8c2, 0x6f8fe0, 0x9a6bff, 0xc76bb0] },
    Spec { name: "Amber Terminal", dark: true, base: [0x100b05, 0x160f07, 0x24190b, 0x3a2810, 0xffb54a, 0x8a5f25, 0xffcc66], acc: [0xff6a3d, 0xff9a3d, 0xffd27a, 0xd9c35a, 0xe8c38a, 0xffb870, 0xe09a6a, 0xff8a5c] },
    Spec { name: "Green Phosphor", dark: true, base: [0x030a03, 0x061006, 0x0b1d0b, 0x133113, 0x6dff8b, 0x2f8f45, 0xa8ffb8], acc: [0xff6b6b, 0xc8ff6b, 0xe6ff8a, 0x6dff8b, 0x7affd6, 0x5ce6a0, 0x9cffc8, 0xc2ffd0] },
    // Light
    Spec { name: "Solarized Light", dark: false, base: [0xfdf6e3, 0xeee8d5, 0xe4ddc8, 0xd3cbb7, 0x586e75, 0x93a1a1, 0x268bd2], acc: [0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682] },
    Spec { name: "Gruvbox Light", dark: false, base: [0xfbf1c7, 0xf2e5bc, 0xebdbb2, 0xd5c4a1, 0x3c3836, 0x928374, 0xb57614], acc: [0x9d0006, 0xaf3a03, 0xb57614, 0x79740e, 0x427b58, 0x076678, 0x8f3f71, 0xb16286] },
    Spec { name: "Catppuccin Latte", dark: false, base: [0xeff1f5, 0xe6e9ef, 0xccd0da, 0xbcc0cc, 0x4c4f69, 0x8c8fa1, 0x8839ef], acc: [0xd20f39, 0xfe640b, 0xdf8e1d, 0x40a02b, 0x179299, 0x1e66f5, 0x8839ef, 0xea76cb] },
    Spec { name: "Rosé Pine Dawn", dark: false, base: [0xfaf4ed, 0xfffaf3, 0xf2e9e1, 0xdfdad9, 0x575279, 0x9893a5, 0x907aa9], acc: [0xb4637a, 0xd7827e, 0xea9d34, 0x4f8a5b, 0x56949f, 0x286983, 0x907aa9, 0xc65d8b] },
    Spec { name: "Everforest Light", dark: false, base: [0xfdf6e3, 0xf4f0d9, 0xefebd4, 0xe0dcc7, 0x5c6a72, 0x939f91, 0x8da101], acc: [0xf85552, 0xf57d26, 0xdfa000, 0x8da101, 0x35a77c, 0x3a94c5, 0xdf69ba, 0xe66fa0] },
    Spec { name: "Ayu Light", dark: false, base: [0xfafafa, 0xf3f4f5, 0xe7e8e9, 0xd9d8d7, 0x5c6166, 0x8a9199, 0xff9940], acc: [0xf07171, 0xfa8d3e, 0xf2ae49, 0x86b300, 0x4cbf99, 0x399ee6, 0xa37acc, 0xe65050] },
    Spec { name: "Virtue", dark: false, base: [0xf8f4e8, 0xefe8d4, 0xe4dcc4, 0xcfc4a6, 0x2b2a33, 0x8c8574, 0x1f5fbf], acc: [0xb3261e, 0xc2621a, 0xa87b00, 0x2e7d32, 0x00838f, 0x1f5fbf, 0x6a3fa0, 0xad1457] },
    Spec { name: "Parchment", dark: false, base: [0xf4ecd8, 0xebe0c6, 0xe0d3b4, 0xcbbb96, 0x3b2f22, 0x8b7a5e, 0x8b4513], acc: [0xa3261b, 0xb5541c, 0x8a6d00, 0x4d6b1f, 0x2f6f6a, 0x2e4f8a, 0x6b3d7a, 0x9c3d63] },
    Spec { name: "Sepia", dark: false, base: [0xefe3cf, 0xe6d7bd, 0xdccaa8, 0xc4ad86, 0x433422, 0x8f7a5c, 0x9c5b2e], acc: [0xa83a2a, 0xb8642a, 0x8f6a10, 0x5a7230, 0x3a7470, 0x3c5a8c, 0x74487e, 0xa0466e] },
    Spec { name: "Paper", dark: false, base: [0xffffff, 0xf2f2f2, 0xe6e6e6, 0xbdbdbd, 0x000000, 0x666666, 0x0050b3], acc: [0xc00000, 0xb35900, 0x806600, 0x00731f, 0x006d77, 0x0050b3, 0x6b00b3, 0xb3006b] },
];

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
    fn theme_names_unique() {
        let all = presets();
        assert!(all.len() >= 40);
        let mut names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len());
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
