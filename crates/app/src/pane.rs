//! A dockable pane: either a filtered journal or the People table.

use eframe::egui::{
    self,
    containers::menu::{MenuButton, MenuConfig},
    Align, Color32, CornerRadius, Layout, PopupCloseBehavior, RichText, Sense, Stroke, Vec2,
};
use rustc_hash::FxHashMap;
use uoj_core::classify::combat_number;
use uoj_core::{flags, time, Channel, ChannelSet, Filter, PersonKind, Query, Store, View};

use crate::config::{PaneConfig, PaneKind, Template};
use crate::logview::{plain_line, LogAction, LogState, LogView, RowStyle};
use crate::theme::Palette;

/// Requests a pane makes of the app.
pub enum PaneAction {
    OpenPane(PaneConfig),
    AddHighlight(String),
    /// The pane's saved configuration changed (persist it).
    ConfigChanged,
}

#[derive(Default)]
struct CombatStats {
    upto: usize,
    dealt: i64,
    taken: i64,
    healed: i64,
    hits: u32,
    biggest: i64,
    targets: FxHashMap<u32, i64>,
}

impl CombatStats {
    fn update(&mut self, store: &Store, rows: &[u32]) {
        if rows.len() < self.upto {
            *self = CombatStats::default();
        }
        for &id in &rows[self.upto..] {
            let e = store.entry(id);
            if e.channel != Channel::Combat {
                continue;
            }
            let Some(n) = combat_number(store.text(e)) else {
                continue;
            };
            if n > 0 {
                self.healed += n;
            } else if e.has(flags::INCOMING) {
                self.taken += -n;
            } else {
                self.dealt += -n;
                self.hits += 1;
                self.biggest = self.biggest.max(-n);
                *self.targets.entry(e.speaker).or_default() += -n;
            }
        }
        self.upto = rows.len();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PeopleSort {
    Name,
    Kind,
    Guild,
    Seen,
    Said,
}

struct PeopleState {
    sort: PeopleSort,
    desc: bool,
    kinds: [bool; 5],
    cache_key: (u64, u64, String, PeopleSort, bool, [bool; 5]),
    rows: Vec<String>,
}

impl Default for PeopleState {
    fn default() -> Self {
        PeopleState {
            sort: PeopleSort::Seen,
            desc: true,
            kinds: [true; 5],
            cache_key: (
                u64::MAX,
                0,
                String::new(),
                PeopleSort::Seen,
                true,
                [true; 5],
            ),
            rows: Vec::new(),
        }
    }
}

fn kind_index(k: PersonKind) -> usize {
    match k {
        PersonKind::Player => 0,
        PersonKind::Npc => 1,
        PersonKind::Pet => 2,
        PersonKind::Creature => 3,
        PersonKind::Unknown => 4,
    }
}

const KIND_ORDER: [PersonKind; 5] = [
    PersonKind::Player,
    PersonKind::Npc,
    PersonKind::Pet,
    PersonKind::Creature,
    PersonKind::Unknown,
];

pub struct Pane {
    pub cfg: PaneConfig,
    pub search: String,
    pub view: View,
    pub log: LogState,
    filter: Filter,
    filter_dirty: bool,
    focus_search: bool,
    combat: CombatStats,
    people: PeopleState,
    min_id: u32,
}

/// Everything a pane needs from the app to draw itself.
pub struct PaneCtx<'a> {
    pub store: &'a Store,
    pub style: &'a RowStyle<'a>,
    pub characters: &'a [String],
    /// Lines below this id were cleared by the user.
    pub min_id: u32,
}

impl Pane {
    pub fn new(cfg: PaneConfig) -> Pane {
        let mut p = Pane {
            cfg,
            search: String::new(),
            view: View::new(),
            log: LogState::default(),
            filter: Filter::default(),
            filter_dirty: true,
            focus_search: false,
            combat: CombatStats::default(),
            people: PeopleState::default(),
            min_id: 0,
        };
        p.rebuild_filter();
        p
    }

    pub fn title(&self) -> String {
        if self.search.trim().is_empty() {
            self.cfg.title.clone()
        } else {
            format!("{} •", self.cfg.title)
        }
    }

    pub fn focus_search(&mut self) {
        self.focus_search = true;
    }

    /// Re-parse queries after the config or search text changed.
    pub fn rebuild_filter(&mut self) {
        self.filter = Filter {
            channels: self.cfg.channels,
            characters: self
                .cfg
                .characters
                .iter()
                .map(|c| c.to_lowercase())
                .collect(),
            show_dups: self.cfg.show_dups,
            base: Query::parse(&self.cfg.filter),
            search: Query::parse(&self.search),
            min_id: self.min_id,
        };
        self.view.invalidate();
        self.combat = CombatStats::default();
        self.filter_dirty = false;
    }

    /// Bring the row list up to date with the store (cheap when nothing changed).
    pub fn sync(&mut self, store: &Store, min_id: u32) {
        if self.cfg.kind != PaneKind::Log {
            return;
        }
        if min_id != self.min_id {
            self.min_id = min_id;
            self.filter_dirty = true;
        }
        if self.filter_dirty {
            self.rebuild_filter();
        }
        let res = self.view.sync(store, &self.filter);
        if res.rebuilt {
            self.log.reanchor(&self.view.rows);
            self.combat = CombatStats::default();
        }
        if res.appended > 0 {
            self.log.rows_appended(res.appended);
        }
        if self.cfg.combat_summary {
            self.combat.update(store, &self.view.rows);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &PaneCtx<'_>) -> Vec<PaneAction> {
        match self.cfg.kind {
            PaneKind::Log => self.log_ui(ui, cx),
            PaneKind::People => self.people_ui(ui, cx),
        }
    }

    fn search_box(&mut self, ui: &mut egui::Ui, pal: &Palette, hint: &str, width: f32) -> bool {
        // Errors of the last applied query; re-parsing here would recompile regexes every frame.
        let err = match self.cfg.kind {
            PaneKind::Log => self.filter.search.error().map(str::to_string),
            PaneKind::People => None,
        };
        let mut edit = egui::TextEdit::singleline(&mut self.search)
            .hint_text(hint)
            .desired_width(width)
            .margin(Vec2::new(6.0, 3.0));
        if err.is_some() {
            edit = edit.text_color(pal.damage_taken);
        }
        let resp = ui.add(edit);
        if self.focus_search {
            resp.request_focus();
            self.focus_search = false;
        }
        let escaped = (resp.has_focus() || resp.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escaped {
            self.search.clear();
        }
        let resp = match &err {
            Some(e) => resp.on_hover_text(format!("Problem: {e}")),
            None => resp.on_hover_text(SEARCH_HELP),
        };
        resp.changed() || escaped
    }

    fn log_ui(&mut self, ui: &mut egui::Ui, cx: &PaneCtx<'_>) -> Vec<PaneAction> {
        let mut out = Vec::new();
        let pal = cx.style.palette;
        self.sync(cx.store, cx.min_id);

        // ---- header ------------------------------------------------------------
        let mut changed_search = false;
        let mut changed_cfg = false;
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                changed_cfg |= self.options_menu(ui, cx, &mut out);
                changed_cfg |= self.channel_menu(ui, cx);
                ui.label(
                    RichText::new(fmt_count(self.view.len()))
                        .color(pal.dim)
                        .small(),
                )
                .on_hover_text("lines shown in this pane");
                let w = ui.available_width();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    changed_search |= self.search_box(
                        ui,
                        pal,
                        "Filter…  (try: from:name  ch:guild  -word  red|pk)",
                        w,
                    );
                });
            });
        });
        if self.cfg.chip_bar {
            changed_cfg |= self.chip_bar(ui, pal, cx.store);
        }
        if self.cfg.combat_summary {
            self.combat_bar(ui, pal, cx.store);
        }
        ui.add_space(2.0);

        if changed_search || changed_cfg {
            self.rebuild_filter();
            self.sync(cx.store, cx.min_id);
        }
        if changed_cfg {
            out.push(PaneAction::ConfigChanged);
        }

        // ---- log -------------------------------------------------------------------
        let view = LogView {
            store: cx.store,
            rows: &self.view.rows,
            style: cx.style,
            queries: [&self.filter.search, &self.filter.base],
            id: ui.id().with(("log", self.cfg.id)),
        };
        for action in view.show(ui, &mut self.log) {
            match action {
                LogAction::SearchSpeaker(name) => {
                    self.search = format!("from:\"{name}\"");
                    self.rebuild_filter();
                }
                LogAction::ExcludeSpeaker(name) => {
                    let term = format!("-from:\"{name}\"");
                    if !self.search.contains(&term) {
                        if !self.search.trim().is_empty() {
                            self.search.push(' ');
                        }
                        self.search.push_str(&term);
                    }
                    self.rebuild_filter();
                }
                LogAction::OpenSpeakerPane(name) => out.push(PaneAction::OpenPane(PaneConfig {
                    title: name.clone(),
                    filter: format!("from:\"{name}\""),
                    ..Template::All.make(0)
                })),
                LogAction::Highlight(word) => out.push(PaneAction::AddHighlight(word)),
            }
        }
        out
    }

    fn channel_menu(&mut self, ui: &mut egui::Ui, cx: &PaneCtx<'_>) -> bool {
        let pal = cx.style.palette;
        let mut changed = false;
        let label = if self.cfg.channels.is_all() {
            "All channels".to_string()
        } else if self.cfg.channels.len() == 1 {
            self.cfg
                .channels
                .iter()
                .next()
                .map(|c| c.label().to_string())
                .unwrap_or_default()
        } else {
            format!("{} channels", self.cfg.channels.len())
        };
        let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        MenuButton::new(format!("{label} ⏷"))
            .config(config)
            .ui(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("All").clicked() {
                        self.cfg.channels = ChannelSet::ALL;
                        changed = true;
                    }
                    if ui.small_button("None").clicked() {
                        self.cfg.channels = ChannelSet::EMPTY;
                        changed = true;
                    }
                    if ui.small_button("Chat").clicked() {
                        use Channel::*;
                        self.cfg.channels =
                            ChannelSet::of(&[Speech, Emote, Guild, Alliance, Party]);
                        changed = true;
                    }
                    if ui.small_button("No spam").clicked() {
                        use Channel::*;
                        let mut c = ChannelSet::ALL;
                        for x in [Client, World, Names, Items, Skill] {
                            c.remove(x);
                        }
                        self.cfg.channels = c;
                        changed = true;
                    }
                });
                ui.separator();
                let counts = cx.store.channel_counts();
                for c in Channel::ALL {
                    let mut on = self.cfg.channels.contains(c);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.5, pal.channel(c));
                        if ui
                            .checkbox(&mut on, c.label())
                            .on_hover_text(c.description())
                            .changed()
                        {
                            self.cfg.channels.set(c, on);
                            changed = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(fmt_count(counts[c.index()]))
                                    .color(pal.dim)
                                    .small(),
                            );
                        });
                    });
                }
            });
        changed
    }

    fn options_menu(
        &mut self,
        ui: &mut egui::Ui,
        cx: &PaneCtx<'_>,
        out: &mut Vec<PaneAction>,
    ) -> bool {
        let mut changed = false;
        let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        MenuButton::new("☰").config(config).ui(ui, |ui| {
            ui.set_min_width(260.0);
            ui.label("Pane title");
            if ui.text_edit_singleline(&mut self.cfg.title).changed() {
                out.push(PaneAction::ConfigChanged);
            }
            ui.add_space(4.0);
            ui.label("Saved filter (always applied)");
            let base_err = self.filter.base.error().map(str::to_string);
            let r = ui.add(egui::TextEdit::singleline(&mut self.cfg.filter).hint_text("e.g. -from:Razor is:mention"));
            if r.changed() {
                changed = true;
            }
            if let Some(e) = base_err {
                ui.colored_label(cx.style.palette.damage_taken, format!("Problem: {e}"));
            }
            ui.separator();
            if !cx.characters.is_empty() {
                ui.label("Characters");
                let mut all = self.cfg.characters.is_empty();
                if ui.checkbox(&mut all, "All characters").changed() && all {
                    self.cfg.characters.clear();
                    changed = true;
                }
                for ch in cx.characters {
                    let mut on = self.cfg.characters.iter().any(|c| c == ch);
                    if ui.checkbox(&mut on, ch).changed() {
                        if on {
                            self.cfg.characters.push(ch.clone());
                        } else {
                            self.cfg.characters.retain(|c| c != ch);
                        }
                        changed = true;
                    }
                }
                ui.separator();
            }
            changed |= ui
                .checkbox(&mut self.cfg.show_dups, "Show duplicates from other clients")
                .on_hover_text("When several clients are logged in, guild/alliance chat arrives once per client.")
                .changed();
            changed |= ui.checkbox(&mut self.cfg.chip_bar, "Channel toggle bar").changed();
            changed |= ui.checkbox(&mut self.cfg.combat_summary, "Damage summary").changed();
            ui.separator();
            if ui.button("Copy all lines in this pane").clicked() {
                let mut s = String::new();
                for &id in &self.view.rows {
                    s.push_str(&plain_line(cx.store, cx.store.entry(id)));
                    s.push('\n');
                }
                ui.ctx().copy_text(s);
                ui.close();
            }
            if ui.button("Export lines to file…").clicked() {
                ui.close();
                let name = format!("{}.txt", self.cfg.title.replace(['/', '\\', ':'], "_"));
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                    let mut s = String::new();
                    for &id in &self.view.rows {
                        s.push_str(&plain_line(cx.store, cx.store.entry(id)));
                        s.push('\n');
                    }
                    let _ = std::fs::write(path, s);
                }
            }
            if ui.button("Duplicate pane").clicked() {
                let mut c = self.cfg.clone();
                c.title = format!("{} (copy)", c.title);
                out.push(PaneAction::OpenPane(c));
                ui.close();
            }
        });
        changed
    }

    fn chip_bar(&mut self, ui: &mut egui::Ui, pal: &Palette, _store: &Store) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for c in Channel::ALL {
                if c == Channel::Client && !self.cfg.channels.contains(c) {
                    continue;
                }
                let on = self.cfg.channels.contains(c);
                let col = pal.channel(c);
                let text = RichText::new(c.label()).size(11.5).color(if on {
                    pal.background
                } else {
                    col
                });
                let btn = egui::Button::new(text)
                    .fill(if on { col } else { Color32::TRANSPARENT })
                    .stroke(Stroke::new(
                        1.0,
                        col.gamma_multiply(if on { 1.0 } else { 0.5 }),
                    ))
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(0.0, 18.0));
                let r = ui
                    .add(btn)
                    .on_hover_text(format!("{}\nRight-click: show only this", c.description()));
                if r.clicked() {
                    self.cfg.channels.toggle(c);
                    changed = true;
                }
                if r.secondary_clicked() {
                    self.cfg.channels = ChannelSet::of(&[c]);
                    changed = true;
                }
            }
        });
        changed
    }

    fn combat_bar(&mut self, ui: &mut egui::Ui, pal: &Palette, store: &Store) {
        let s = &self.combat;
        let mut reset = false;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("Dealt").color(pal.dim).small());
            ui.label(
                RichText::new(fmt_count(s.dealt as usize))
                    .color(pal.damage_dealt)
                    .strong(),
            );
            if s.hits > 0 {
                ui.label(
                    RichText::new(format!(
                        "{} hits · avg {} · max {}",
                        s.hits,
                        s.dealt / s.hits as i64,
                        s.biggest
                    ))
                    .color(pal.dim)
                    .small(),
                );
            }
            ui.label(RichText::new("Taken").color(pal.dim).small());
            ui.label(
                RichText::new(fmt_count(s.taken as usize))
                    .color(pal.damage_taken)
                    .strong(),
            );
            if s.healed > 0 {
                ui.label(RichText::new("Healed").color(pal.dim).small());
                ui.label(
                    RichText::new(fmt_count(s.healed as usize))
                        .color(pal.heal)
                        .strong(),
                );
            }
            if ui
                .small_button("reset")
                .on_hover_text("Restart the totals from now")
                .clicked()
            {
                reset = true;
            }
            let mut top: Vec<(&u32, &i64)> = s.targets.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            if !top.is_empty() {
                let list = top
                    .into_iter()
                    .take(3)
                    .map(|(spk, dmg)| format!("{} {}", store.speaker_by_id(*spk), short_num(*dmg)))
                    .collect::<Vec<_>>()
                    .join(", ");
                ui.label(RichText::new(format!("Top: {list}")).color(pal.dim).small());
            }
        });
        if reset {
            self.combat = CombatStats {
                upto: self.view.rows.len(),
                ..Default::default()
            };
        }
    }

    // ---- People table -------------------------------------------------------------

    fn people_ui(&mut self, ui: &mut egui::Ui, cx: &PaneCtx<'_>) -> Vec<PaneAction> {
        let mut out = Vec::new();
        let pal = cx.style.palette;
        let store = cx.store;
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            for (i, k) in KIND_ORDER.iter().enumerate() {
                let label = match k {
                    PersonKind::Unknown => "Unknown",
                    other => other.label(),
                };
                ui.toggle_value(&mut self.people.kinds[i], label);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(fmt_count(self.people.rows.len()))
                        .color(pal.dim)
                        .small(),
                );
                let w = ui.available_width();
                self.search_box(ui, pal, "Find name or guild…", w);
            });
        });
        ui.add_space(2.0);

        let key = (
            store.epoch(),
            store.people_revision(),
            self.search.to_lowercase(),
            self.people.sort,
            self.people.desc,
            self.people.kinds,
        );
        if key != self.people.cache_key {
            let needle = key.2.clone();
            let mut v: Vec<&uoj_core::Person> = store
                .people()
                .values()
                .filter(|p| self.people.kinds[kind_index(p.kind)])
                .filter(|p| {
                    needle.is_empty()
                        || p.name.to_lowercase().contains(&needle)
                        || p.guild
                            .as_deref()
                            .map(|g| g.to_lowercase().contains(&needle))
                            .unwrap_or(false)
                        || p.title
                            .as_deref()
                            .map(|g| g.to_lowercase().contains(&needle))
                            .unwrap_or(false)
                })
                .collect();
            match self.people.sort {
                PeopleSort::Name => v.sort_by_key(|p| p.name.to_lowercase()),
                PeopleSort::Kind => v.sort_by_key(|p| (kind_index(p.kind), p.name.to_lowercase())),
                PeopleSort::Guild => v.sort_by_key(|p| {
                    (
                        p.guild.clone().unwrap_or_default().to_lowercase(),
                        p.name.to_lowercase(),
                    )
                }),
                PeopleSort::Seen => v.sort_by_key(|p| (p.last_seen, p.name.clone())),
                PeopleSort::Said => v.sort_by_key(|p| (p.said, p.name.clone())),
            }
            if self.people.desc {
                v.reverse();
            }
            self.people.rows = v.into_iter().map(|p| p.name.clone()).collect();
            self.people.cache_key = key;
        }

        let font = cx.style.font.clone();
        let row_h = font.size + 6.0;
        let width = ui.available_width();
        let cols = [0.30f32, 0.11, 0.29, 0.10, 0.10, 0.10];
        let col_w: Vec<f32> = cols.iter().map(|f| (width - 16.0) * f).collect();

        // Header
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            for (i, (name, sort)) in [
                ("Name", Some(PeopleSort::Name)),
                ("Guild", Some(PeopleSort::Guild)),
                ("Title", None),
                ("Kind", Some(PeopleSort::Kind)),
                ("Seen", Some(PeopleSort::Seen)),
                ("Lines", Some(PeopleSort::Said)),
            ]
            .into_iter()
            .enumerate()
            {
                let arrow = match sort {
                    Some(s) if s == self.people.sort => {
                        if self.people.desc {
                            " ⏷"
                        } else {
                            " ⏶"
                        }
                    }
                    _ => "",
                };
                let r = cell(
                    ui,
                    col_w[i],
                    row_h,
                    egui::Label::new(
                        RichText::new(format!("{name}{arrow}"))
                            .color(pal.dim)
                            .strong(),
                    )
                    .sense(Sense::click()),
                );
                if let (true, Some(s)) = (r.clicked(), sort) {
                    if self.people.sort == s {
                        self.people.desc = !self.people.desc;
                    } else {
                        self.people.sort = s;
                        self.people.desc = matches!(s, PeopleSort::Seen | PeopleSort::Said);
                    }
                }
            }
        });
        ui.separator();

        let rows = &self.people.rows;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, rows.len(), |ui, range| {
                for name in &rows[range] {
                    let Some(p) = store.person(name) else {
                        continue;
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            ui.set_min_height(row_h);
                            ui.add_space(6.0);
                            let name_col = if p.is_self {
                                pal.accent
                            } else if cx.style.color_names {
                                pal.name_color(&p.name)
                            } else {
                                pal.name
                            };
                            let mut name_txt =
                                RichText::new(&p.name).color(name_col).font(font.clone());
                            if p.is_self {
                                name_txt = name_txt.strong();
                            }
                            cell(ui, col_w[0], row_h, egui::Label::new(name_txt));
                            let guild = match (&p.guild, &p.guild_title) {
                                (Some(g), _) => g.clone(),
                                _ => String::new(),
                            };
                            cell(
                                ui,
                                col_w[1],
                                row_h,
                                egui::Label::new(
                                    RichText::new(guild)
                                        .color(pal.channel(Channel::Guild))
                                        .font(font.clone()),
                                ),
                            );
                            let title = [
                                p.title.clone(),
                                p.guild_title.clone(),
                                p.status.clone().map(|s| format!("({s})")),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                            cell(
                                ui,
                                col_w[2],
                                row_h,
                                egui::Label::new(
                                    RichText::new(title).color(pal.text).font(font.clone()),
                                ),
                            );
                            cell(
                                ui,
                                col_w[3],
                                row_h,
                                egui::Label::new(
                                    RichText::new(p.kind.label())
                                        .color(pal.dim)
                                        .font(font.clone()),
                                ),
                            );
                            cell(
                                ui,
                                col_w[4],
                                row_h,
                                egui::Label::new(
                                    RichText::new(time::hm(p.last_seen))
                                        .color(pal.dim)
                                        .font(font.clone()),
                                ),
                            );
                            cell(
                                ui,
                                col_w[5],
                                row_h,
                                egui::Label::new(
                                    RichText::new(fmt_count(p.said as usize))
                                        .color(pal.dim)
                                        .font(font.clone()),
                                ),
                            );
                        })
                        .response;
                    let resp = resp.interact(Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(
                            resp.rect,
                            CornerRadius::ZERO,
                            pal.surface.gamma_multiply(0.3),
                        );
                    }
                    let tip = person_tooltip(p);
                    let resp = resp.on_hover_text(tip);
                    if resp.double_clicked() {
                        out.push(open_person(&p.name));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Show their lines in a new pane").clicked() {
                            out.push(open_person(&p.name));
                            ui.close();
                        }
                        if ui.button("Copy name").clicked() {
                            ui.ctx().copy_text(p.name.clone());
                            ui.close();
                        }
                        if ui.button("Highlight name").clicked() {
                            out.push(PaneAction::AddHighlight(p.name.clone()));
                            ui.close();
                        }
                    });
                }
            });
        out
    }
}

/// A fixed-width, left-aligned, truncating table cell.
fn cell(ui: &mut egui::Ui, width: f32, height: f32, label: egui::Label) -> egui::Response {
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, height));
            ui.add(label.truncate())
        },
    )
    .inner
}

fn open_person(name: &str) -> PaneAction {
    PaneAction::OpenPane(PaneConfig {
        title: name.to_string(),
        filter: format!("from:\"{name}\""),
        ..Template::All.make(0)
    })
}

fn person_tooltip(p: &uoj_core::Person) -> String {
    let mut s = format!("{}  ({})", p.name, p.kind.label());
    if let Some(g) = &p.guild {
        s += &format!("\nGuild: {g}");
        if let Some(t) = &p.guild_title {
            s += &format!(" — {t}");
        }
    }
    if let Some(t) = &p.title {
        s += &format!("\nTitle: {t}");
    }
    s += &format!(
        "\nFirst seen {}\nLast seen {}\nName shown {}× · spoke {}×",
        time::ymd_hm(p.first_seen),
        time::ymd_hm(p.last_seen),
        p.shown,
        p.said
    );
    if p.damage_taken != 0 {
        s += &format!("\nCombat numbers on them: {}", p.damage_taken);
    }
    s += "\n\nDouble-click to show their lines";
    s
}

pub const SEARCH_HELP: &str = "Search syntax\n\
  red pk          all words (case-insensitive)\n\
  Red             upper-case makes a word case-sensitive\n\
  \"level 3 gate\"  exact phrase\n\
  red|pk          either word (also: red OR pk)\n\
  -world          exclude\n\
  from:quill      by speaker  (also @quill, from:\"oswin pike\")\n\
  ch:guild,ally   channel\n\
  char:thorne     lines from that character's client\n\
  is:mention  is:self  is:dup  is:incoming\n\
  /\\d+ reds?/     regular expression\n\
Esc clears the box.";

pub fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn short_num(n: i64) -> String {
    let a = n.unsigned_abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    if a >= 1_000_000.0 {
        format!("{sign}{:.1}M", a / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("{sign}{:.0}k", a / 1000.0)
    } else if a >= 1000.0 {
        format!("{sign}{:.1}k", a / 1000.0)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(1234567), "1,234,567");
        assert_eq!(short_num(3140), "3.1k");
        assert_eq!(short_num(12000), "12k");
        assert_eq!(short_num(-438), "-438");
    }
}
