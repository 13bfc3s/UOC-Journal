//! The application: owns the store, the watcher thread, the dock layout and
//! all panes, and draws menus, status bar and windows around them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, FontData, FontDefinitions, FontFamily, FontId, Key, KeyboardShortcut, Layout,
    Modifiers, RichText, TextStyle, UserAttentionType, ViewportCommand, WindowLevel,
};
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, NodeIndex, NodePath, TabViewer};
use uoj_core::watcher::{self, Command, Event, Status, WatchConfig, Watcher};
use uoj_core::{Channel, Store};

use crate::config::{self, HighlightRule, PaneConfig, Settings, Template, TimeFormat};
use crate::logview::{Highlight, RowStyle};
use crate::pane::{fmt_count, Pane, PaneAction, PaneCtx, SEARCH_HELP};
use crate::settings_ui::SettingsUi;
use crate::theme::{Palette, Rgb};

/// Distinct colours handed to new characters, in order.
const CHIP_COLORS: [u32; 12] = [
    0xffd166, 0x2ec4b6, 0xef476f, 0x4cc9f0, 0x06d6a0, 0xf78c6b, 0x9b5de5, 0xf15bb5, 0x8ac926,
    0xfee440, 0x00bbf9, 0xff924c,
];

pub struct JournalApp {
    pub settings: Settings,
    pub config_dir: PathBuf,
    pub store: Store,
    pub watcher: Watcher,
    pub status: Status,
    dock: DockState<u64>,
    panes: BTreeMap<u64, Pane>,
    pub palette: Palette,
    pub highlights: Vec<Highlight>,
    pub highlight_errors: Vec<String>,
    pub settings_ui: SettingsUi,
    show_help: bool,
    show_about: bool,
    pub show_welcome: bool,
    pub detected: Option<Vec<PathBuf>>,
    detecting: Option<std::sync::mpsc::Receiver<Vec<PathBuf>>>,
    pub settings_dirty: bool,
    settings_changed_at: Instant,
    last_layout: String,
    last_layout_check: Instant,
    pub notice: Option<(String, Instant)>,
    applied_fonts: Option<(Option<PathBuf>, f32)>,
    /// A user font is loaded into the "journal" font family.
    journal_font: bool,
    /// Lines with a lower id are hidden ("Clear").
    clear_mark: u32,
    pending_alert: bool,
    /// Theme shown while hovering a theme list (not saved until clicked).
    pub theme_preview: Option<String>,
    /// Set by any theme list drawn this frame.
    pub theme_list_shown: bool,
    pub theme_hovered: Option<String>,
    /// Font shown while hovering the font list (`Some(None)` = built-in).
    pub font_preview: Option<Option<PathBuf>>,
    pub font_list_shown: bool,
    pub font_hovered: Option<Option<PathBuf>>,
    /// Installed fonts (scanned in the background on first use).
    pub system_fonts: Option<Vec<crate::fonts::FontEntry>>,
    fonts_rx: Option<std::sync::mpsc::Receiver<Vec<crate::fonts::FontEntry>>>,
    characters: Vec<String>,
}

impl JournalApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config_dir = config::config_dir();
        let (mut settings, load_err) = config::load(&config_dir);

        let ctx = cc.egui_ctx.clone();
        let watcher = Watcher::spawn(watch_config(&settings), move || ctx.request_repaint());

        if settings.panes.is_empty() {
            settings.panes = default_panes(&mut settings);
        }
        let panes: BTreeMap<u64, Pane> = settings
            .panes
            .iter()
            .map(|p| (p.id, Pane::new(p.clone())))
            .collect();
        let dock = load_layout(&config_dir, &panes).unwrap_or_else(|| default_layout(&panes));

        let palette = Palette::from_theme(&settings.current_theme());
        let show_welcome = settings.journal_folder.is_none();
        let mut app = JournalApp {
            config_dir,
            store: Store::new(),
            watcher,
            status: Status::default(),
            last_layout: serde_json::to_string(&dock).unwrap_or_default(),
            dock,
            panes,
            palette,
            highlights: Vec::new(),
            highlight_errors: Vec::new(),
            settings_ui: SettingsUi::default(),
            show_help: false,
            show_about: false,
            show_welcome,
            detected: None,
            detecting: None,
            settings_dirty: false,
            settings_changed_at: Instant::now(),
            last_layout_check: Instant::now(),
            notice: load_err.map(|e| (e, Instant::now())),
            applied_fonts: None,
            journal_font: false,
            clear_mark: 0,
            pending_alert: false,
            theme_preview: None,
            theme_list_shown: false,
            theme_hovered: None,
            font_preview: None,
            font_list_shown: false,
            font_hovered: None,
            system_fonts: None,
            fonts_rx: None,
            characters: Vec::new(),
            settings,
        };
        app.compile_highlights();
        app.apply_style(&cc.egui_ctx);
        // Open a window at startup (handy for docs/screenshots): settings, help, welcome.
        match std::env::var("UOC_JOURNAL_OPEN").as_deref() {
            Ok("settings") => app.settings_ui.open = true,
            Ok("appearance") => {
                app.settings_ui.open = true;
                app.settings_ui.tab = crate::settings_ui::Tab::Appearance;
            }
            Ok("channels") => {
                app.settings_ui.open = true;
                app.settings_ui.tab = crate::settings_ui::Tab::Channels;
            }
            Ok("characters") => {
                app.settings_ui.open = true;
                app.settings_ui.tab = crate::settings_ui::Tab::Characters;
            }
            Ok("help") => app.show_help = true,
            Ok("welcome") => app.show_welcome = true,
            _ => {}
        }
        if app.settings.always_on_top {
            cc.egui_ctx
                .send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
        }
        app
    }

    // ---- settings plumbing ------------------------------------------------------

    pub fn mark_dirty(&mut self) {
        self.settings_dirty = true;
        self.settings_changed_at = Instant::now();
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.notice = Some((msg.into(), Instant::now()));
    }

    pub fn compile_highlights(&mut self) {
        self.highlights.clear();
        self.highlight_errors.clear();
        for (i, r) in self.settings.highlights.iter().enumerate() {
            if !r.enabled || r.pattern.trim().is_empty() {
                continue;
            }
            match Highlight::compile(r) {
                Ok(h) => self.highlights.push(h),
                Err(e) => self
                    .highlight_errors
                    .push(format!("Highlight #{}: {e}", i + 1)),
            }
        }
    }

    /// Re-send folder/history/rules to the watcher (reloads when they changed).
    pub fn reconfigure_watcher(&mut self) {
        self.watcher
            .send(Command::Configure(watch_config(&self.settings)));
    }

    /// Search for JournalLogs folders on a background thread and show the picker.
    pub fn start_detect(&mut self, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(watcher::detect_journal_dirs());
            ctx.request_repaint();
        });
        self.detecting = Some(rx);
        self.detected = None;
        self.show_welcome = true;
    }

    pub fn set_folder(&mut self, folder: PathBuf) {
        self.settings.journal_folder = Some(folder);
        self.settings.onboarded = true;
        self.show_welcome = false;
        self.clear_mark = 0;
        self.reconfigure_watcher();
        self.mark_dirty();
    }

    /// Give every known character a stored chip (initials + an unused colour).
    /// Returns true if any chip was added or completed.
    pub fn ensure_chips(&mut self) -> bool {
        let mut changed = false;
        for name in self.store.characters() {
            if !self.settings.chips.iter().any(|c| c.name == name) {
                self.settings.chips.push(crate::config::CharacterChip {
                    label: crate::logview::initials(&name),
                    name: name.clone(),
                    ..Default::default()
                });
                changed = true;
            }
        }
        for i in 0..self.settings.chips.len() {
            if self.settings.chips[i].color.is_none() {
                let used: Vec<Rgb> = self.settings.chips.iter().filter_map(|c| c.color).collect();
                let pick = CHIP_COLORS
                    .iter()
                    .map(|&c| Rgb::hex(c))
                    .find(|c| !used.contains(c))
                    .unwrap_or_else(|| {
                        Rgb::from_color(self.palette.name_color(&self.settings.chips[i].name))
                    });
                self.settings.chips[i].color = Some(pick);
                changed = true;
            }
        }
        if changed {
            self.refresh_chips();
            self.mark_dirty();
        }
        changed
    }

    /// Installed fonts, starting a background scan on first call. `None` while scanning.
    pub fn system_fonts(&mut self, ctx: &egui::Context) -> Option<&[crate::fonts::FontEntry]> {
        if let Some(rx) = &self.fonts_rx {
            if let Ok(list) = rx.try_recv() {
                self.system_fonts = Some(list);
                self.fonts_rx = None;
            }
        } else if self.system_fonts.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(crate::fonts::scan_system_fonts());
                ctx.request_repaint();
            });
            self.fonts_rx = Some(rx);
        }
        self.system_fonts.as_deref()
    }

    /// Copy chip settings into the palette used for painting.
    pub fn refresh_chips(&mut self) {
        self.palette.chips.clear();
        for chip in &self.settings.chips {
            let label: String = chip.label.trim().chars().take(3).collect();
            let label = if label.is_empty() {
                crate::logview::initials(&chip.name)
            } else {
                label
            };
            let color = chip
                .color
                .map(|c| c.color())
                .unwrap_or_else(|| self.palette.name_color(&chip.name));
            self.palette.chips.insert(chip.name.clone(), (label, color));
        }
    }

    pub fn apply_style(&mut self, ctx: &egui::Context) {
        let theme = self
            .theme_preview
            .as_ref()
            .and_then(|n| {
                self.settings
                    .all_themes()
                    .into_iter()
                    .find(|t| &t.name == n)
            })
            .unwrap_or_else(|| self.settings.current_theme());
        self.palette = Palette::from_theme(&theme);
        self.refresh_chips();
        let visuals = self.palette.visuals();
        ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
        ctx.set_visuals_of(egui::Theme::Light, visuals);
        let size = self.settings.font_size.clamp(8.0, 40.0);
        ctx.all_styles_mut(|s| {
            s.text_styles
                .insert(TextStyle::Body, FontId::proportional(size));
            s.text_styles
                .insert(TextStyle::Button, FontId::proportional(size));
            s.text_styles.insert(
                TextStyle::Small,
                FontId::proportional((size * 0.8).max(8.0)),
            );
            s.text_styles
                .insert(TextStyle::Heading, FontId::proportional(size * 1.35));
            s.text_styles
                .insert(TextStyle::Monospace, FontId::monospace(size));
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(6.0, 2.0);
        });
        let font_path = match &self.font_preview {
            Some(p) => p.clone(),
            None => self.settings.font_path.clone(),
        };
        let fonts_key = (font_path.clone(), 0.0);
        if self.applied_fonts.as_ref() != Some(&fonts_key) {
            // The chosen font only applies to journal text (family "journal");
            // menus and the settings window keep the default font so lists don't
            // change size while you browse fonts.
            let mut defs = FontDefinitions::default();
            let mut journal = defs
                .families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default();
            self.journal_font = false;
            if let Some(path) = &font_path {
                match std::fs::read(path) {
                    Ok(bytes) => {
                        defs.font_data.insert(
                            "user".into(),
                            std::sync::Arc::new(FontData::from_owned(bytes)),
                        );
                        journal.insert(0, "user".into());
                        self.journal_font = true;
                    }
                    Err(e) => self.notify(format!("Could not load font {}: {e}", path.display())),
                }
            }
            defs.families
                .insert(FontFamily::Name("journal".into()), journal);
            ctx.set_fonts(defs);
            self.applied_fonts = Some(fonts_key);
        }
    }

    fn save_now(&mut self) {
        self.settings.panes = self.panes.values().map(|p| p.cfg.clone()).collect();
        if let Err(e) = config::save(&self.config_dir, &self.settings) {
            self.notify(format!("Could not save settings: {e}"));
        }
        self.save_layout();
        self.settings_dirty = false;
    }

    fn save_layout(&mut self) {
        if let Ok(json) = serde_json::to_string(&self.dock) {
            if json != self.last_layout {
                let _ = std::fs::create_dir_all(&self.config_dir);
                let _ = config::write_atomic(
                    &self.config_dir.join(config::LAYOUT_FILE),
                    json.as_bytes(),
                );
                self.last_layout = json;
            }
        }
    }

    // ---- panes -------------------------------------------------------------------

    pub fn open_pane(&mut self, mut cfg: PaneConfig, at: Option<NodePath>) {
        cfg.id = self.settings.alloc_pane_id();
        let id = cfg.id;
        self.panes.insert(id, Pane::new(cfg));
        match at.and_then(|p| self.dock.leaf_mut(p).ok()) {
            Some(leaf) => leaf.append_tab(id),
            None => self.dock.push_to_focused_leaf(id),
        }
        self.mark_dirty();
    }

    fn reset_layout(&mut self) {
        let mut s = Settings {
            next_pane_id: 1,
            ..Default::default()
        };
        let panes = default_panes(&mut s);
        self.settings.next_pane_id = s.next_pane_id;
        self.panes = panes.into_iter().map(|p| (p.id, Pane::new(p))).collect();
        self.dock = default_layout(&self.panes);
        self.mark_dirty();
    }

    fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        let id = self.dock.find_active_focused().map(|(_, t)| *t)?;
        self.panes.get_mut(&id)
    }

    // ---- data ----------------------------------------------------------------------

    fn pump(&mut self, ctx: &egui::Context) {
        let mut got = false;
        let mut reload = false;
        while let Some(ev) = self.watcher.try_recv() {
            match ev {
                Event::Batch(batch) => {
                    let first = self.store.len();
                    let reset = batch.reset;
                    if !self.store.apply(batch) {
                        reload = true;
                        continue;
                    }
                    if reset {
                        self.clear_mark = 0;
                    } else {
                        self.check_alerts(first);
                    }
                    got = true;
                }
                Event::Status(st) => {
                    if st.loading.is_some() {
                        self.status.loading = st.loading;
                    } else {
                        self.status = st;
                    }
                }
            }
        }
        if reload {
            self.watcher.send(Command::Reload);
        }
        if got {
            self.characters = self.store.characters();
            self.ensure_chips();
        }
        if self.pending_alert {
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
            if !focused {
                ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                    UserAttentionType::Informational,
                ));
            }
            self.pending_alert = false;
        }
    }

    fn check_alerts(&mut self, from: usize) {
        if !self.highlights.iter().any(|h| h.alert) {
            return;
        }
        for e in &self.store.entries()[from..] {
            if e.has(uoj_core::flags::DUP) {
                continue;
            }
            let text = self.store.text(e);
            if self
                .highlights
                .iter()
                .any(|h| h.alert && h.applies(e.channel) && h.re.is_match(text))
            {
                self.pending_alert = true;
                return;
            }
        }
    }

    // ---- UI ------------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let find = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
        let settings = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
        let clear = KeyboardShortcut::new(Modifiers::COMMAND, Key::L);
        let reload = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
        let help = KeyboardShortcut::new(Modifiers::NONE, Key::F1);
        if ctx.input_mut(|i| i.consume_shortcut(&find)) {
            if let Some(p) = self.focused_pane_mut() {
                p.focus_search();
            } else if let Some(p) = self.panes.values_mut().next() {
                p.focus_search();
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&settings)) {
            self.settings_ui.open = !self.settings_ui.open;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&clear)) {
            self.clear_mark = self.store.len() as u32;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&reload)) {
            self.clear_mark = 0;
            self.watcher.send(Command::Reload);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&help)) {
            self.show_help = !self.show_help;
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::containers::menu::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Journal", |ui| {
                if ui.button("Choose journal folder…").clicked() {
                    ui.close();
                    self.pick_folder();
                }
                if ui.button("Find journal folders automatically").clicked() {
                    ui.close();
                    let ctx = ui.ctx().clone();
                    self.start_detect(&ctx);
                }
                if ui.button("Open journal file(s)…").clicked() {
                    ui.close();
                    let mut dlg = rfd::FileDialog::new().add_filter("Journal", &["txt"]);
                    if let Some(dir) = &self.settings.journal_folder {
                        dlg = dlg.set_directory(dir);
                    }
                    if let Some(files) = dlg.pick_files() {
                        self.watcher.send(Command::OpenFiles(files));
                    }
                }
                ui.separator();
                if ui.button("Load the whole folder's history").clicked() {
                    ui.close();
                    self.clear_mark = 0;
                    self.watcher.send(Command::LoadAll);
                }
                if ui
                    .add(egui::Button::new("Reload").shortcut_text("F5"))
                    .clicked()
                {
                    ui.close();
                    self.clear_mark = 0;
                    self.watcher.send(Command::Reload);
                }
                if ui
                    .add(
                        egui::Button::new("Clear (hide current lines)").shortcut_text(
                            ui.ctx().format_shortcut(&KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::L,
                            )),
                        ),
                    )
                    .clicked()
                {
                    ui.close();
                    self.clear_mark = self.store.len() as u32;
                }
                if self.clear_mark > 0 && ui.button("Un-clear").clicked() {
                    ui.close();
                    self.clear_mark = 0;
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("New pane", |ui| {
                    for t in Template::MENU {
                        if ui.button(t.label()).clicked() {
                            ui.close();
                            self.open_pane(t.make(0), None);
                        }
                    }
                });
                if ui.button("Reset layout").clicked() {
                    ui.close();
                    self.reset_layout();
                }
                ui.separator();
                let mut changed = false;
                ui.label(RichText::new("Time column").small().weak());
                for f in TimeFormat::ALL {
                    changed |= ui
                        .radio_value(&mut self.settings.time_format, f, f.label())
                        .changed();
                }
                if self.settings.time_format == TimeFormat::Custom
                    && ui.small_button("Edit custom format…").clicked()
                {
                    ui.close();
                    self.settings_ui.open = true;
                    self.settings_ui.tab = crate::settings_ui::Tab::Appearance;
                }
                ui.separator();
                changed |= ui
                    .checkbox(&mut self.settings.show_badges, "Channel badges")
                    .changed();
                changed |= ui
                    .checkbox(&mut self.settings.color_names, "Colour names")
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.character_tags,
                        "Character tags (multi-client)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut self.settings.monospace, "Monospace text")
                    .changed();
                if ui
                    .checkbox(&mut self.settings.always_on_top, "Always on top")
                    .changed()
                {
                    let level = if self.settings.always_on_top {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    };
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::WindowLevel(level));
                    changed = true;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Text size");
                    if ui.button("−").clicked() {
                        self.settings.font_size = (self.settings.font_size - 1.0).max(8.0);
                        changed = true;
                    }
                    ui.label(format!("{:.0}", self.settings.font_size));
                    if ui.button("+").clicked() {
                        self.settings.font_size = (self.settings.font_size + 1.0).min(40.0);
                        changed = true;
                    }
                });
                if changed {
                    let ctx = ui.ctx().clone();
                    self.apply_style(&ctx);
                    self.mark_dirty();
                }
            });
            ui.menu_button("Theme", |ui| {
                let mut pick = None;
                let mut shown = false;
                let mut hovered: Option<String> = None;
                let themes = self.settings.all_themes();
                for (title, dark) in [("Dark themes", true), ("Light themes", false)] {
                    ui.menu_button(title, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(480.0)
                            .show(ui, |ui| {
                                shown = true;
                                for t in themes.iter().filter(|t| t.dark == dark) {
                                    let sel = t.name == self.settings.theme;
                                    let r = ui.radio(sel, &t.name);
                                    if r.hovered() {
                                        hovered = Some(t.name.clone());
                                    }
                                    if r.clicked() {
                                        pick = Some(t.name.clone());
                                    }
                                }
                            });
                    });
                }
                ui.label(
                    RichText::new(format!("Current: {}", self.settings.theme))
                        .small()
                        .weak(),
                );
                ui.separator();
                if ui.button("Edit colours…").clicked() {
                    ui.close();
                    self.settings_ui.open = true;
                    self.settings_ui.tab = crate::settings_ui::Tab::Appearance;
                }
                self.theme_list_shown |= shown;
                if hovered.is_some() {
                    self.theme_hovered = hovered;
                }
                if let Some(name) = pick {
                    self.settings.theme = name;
                    self.theme_preview = None;
                    let ctx = ui.ctx().clone();
                    self.apply_style(&ctx);
                    self.mark_dirty();
                    ui.close();
                }
            });
            if ui.button("Settings").clicked() {
                self.settings_ui.open = !self.settings_ui.open;
            }
            ui.menu_button("Help", |ui| {
                if ui
                    .add(egui::Button::new("Search syntax & shortcuts").shortcut_text("F1"))
                    .clicked()
                {
                    ui.close();
                    self.show_help = true;
                }
                if ui.button("About").clicked() {
                    ui.close();
                    self.show_about = true;
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.live_indicator(ui);
            });
        });
    }

    fn live_indicator(&mut self, ui: &mut egui::Ui) {
        let pal = &self.palette;
        if let Some(msg) = &self.status.loading {
            ui.spinner();
            ui.label(RichText::new(msg).color(pal.dim));
            return;
        }
        let active: Vec<&watcher::FileStatus> =
            self.status.files.iter().filter(|f| f.active).collect();
        if self.settings.journal_folder.is_none() && self.status.files.is_empty() {
            if ui
                .button(RichText::new("Choose journal folder").color(pal.accent))
                .clicked()
            {
                self.show_welcome = true;
            }
            return;
        }
        let (dot, label) = if active.is_empty() {
            (pal.dim, "idle".to_string())
        } else {
            let mut names: Vec<String> =
                active.iter().filter_map(|f| f.character.clone()).collect();
            names.sort();
            names.dedup();
            let who = if names.is_empty() {
                format!("{} file(s)", active.len())
            } else if names.len() > 3 {
                format!("{} characters", names.len())
            } else {
                names.join(", ")
            };
            (pal.heal, format!("live · {who}"))
        };
        let tip = self
            .status
            .files
            .iter()
            .rev()
            .take(20)
            .map(|f| {
                format!(
                    "{} {}  {}  {:.1} KB",
                    if f.active { "•" } else { "○" },
                    f.name,
                    f.character.clone().unwrap_or_default(),
                    f.bytes as f64 / 1024.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        ui.label(RichText::new(label).color(pal.text))
            .on_hover_text(tip.clone());
        status_dot(ui, dot).on_hover_text(tip);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let folder = self
                .settings
                .journal_folder
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "no folder selected".into());
            ui.label(RichText::new(folder).color(pal.dim).small());
            if let Some((msg, at)) = &self.notice {
                if at.elapsed() < Duration::from_secs(8) {
                    ui.label(RichText::new(msg).color(pal.accent).small());
                }
            }
            let errs: Vec<String> = self
                .status
                .errors
                .iter()
                .chain(self.highlight_errors.iter())
                .cloned()
                .collect();
            if !errs.is_empty() {
                ui.label(
                    RichText::new(format!("{} problem(s)", errs.len()))
                        .color(pal.damage_taken)
                        .small(),
                )
                .on_hover_text(errs.join("\n"));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut parts = vec![
                    format!("{} lines", fmt_count(self.store.len())),
                    format!("{} people", fmt_count(self.store.people().len())),
                    format!("{} files", self.status.files.len()),
                ];
                if let Some(ms) = self.status.load_ms {
                    parts.push(format!("loaded in {ms} ms"));
                }
                if self.clear_mark > 0 {
                    parts.push("cleared".into());
                }
                ui.label(RichText::new(parts.join("  ·  ")).color(pal.dim).small());
                if self.settings.character_tags && self.characters.len() > 1 {
                    let mut open_chars = false;
                    for ch in self.characters.iter().rev() {
                        let name = ui.add(
                            egui::Label::new(RichText::new(ch).color(pal.dim).small())
                                .sense(egui::Sense::click()),
                        );
                        let pill = crate::logview::character_pill(ui, &pal, ch);
                        if name.clicked() || pill.clicked() {
                            open_chars = true;
                        }
                        if name.hovered() || pill.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    if open_chars {
                        self.settings_ui.open = true;
                        self.settings_ui.tab = crate::settings_ui::Tab::Characters;
                    }
                }
            });
        });
    }

    fn pick_folder(&mut self) {
        let mut dlg = rfd::FileDialog::new().set_title("Select the JournalLogs folder");
        if let Some(dir) = &self.settings.journal_folder {
            dlg = dlg.set_directory(dir);
        }
        if let Some(dir) = dlg.pick_folder() {
            self.set_folder(dir);
        }
    }

    fn welcome_window(&mut self, ctx: &egui::Context) {
        if !self.show_welcome {
            return;
        }
        if let Some(rx) = &self.detecting {
            if let Ok(found) = rx.try_recv() {
                self.detected = Some(found);
                self.detecting = None;
            }
        } else if self.detected.is_none() {
            self.start_detect(ctx);
        }
        let mut open = true;
        let mut chosen: Option<PathBuf> = None;
        let mut browse = false;
        egui::Window::new("Where are your journal logs?")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .max_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(
                    "UOC Journal reads the journal files your client writes while you play. \
                     In ClassicUO / the Outlands client, enable “Save journal to file” in the options; \
                     files then appear in Data/Client/JournalLogs inside the client folder \
                     (for Wine, Lutris or Proton installs that folder lives inside the prefix's drive_c).",
                );
                ui.add_space(8.0);
                match &self.detected {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Searching native, Wine, Lutris, Bottles and Proton installs…");
                        });
                    }
                    Some(found) if !found.is_empty() => {
                        ui.label(RichText::new("Found these folders (newest first):").strong());
                        for p in found {
                            ui.horizontal(|ui| {
                                if ui.button("Use").clicked() {
                                    chosen = Some(p.clone());
                                }
                                ui.add(egui::Label::new(short_path(p)).truncate())
                                    .on_hover_text(p.display().to_string());
                            });
                        }
                    }
                    _ => {
                        ui.label(RichText::new("No JournalLogs folder found automatically.").color(self.palette.dim));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Browse…").clicked() {
                        browse = true;
                    }
                    if ui.add_enabled(self.detecting.is_none(), egui::Button::new("Search again")).clicked() {
                        self.detected = None;
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Or paste a path:").small());
                let mut typed = self.settings_ui.folder_text.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut typed).desired_width(f32::INFINITY));
                self.settings_ui.folder_text = typed.clone();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !typed.trim().is_empty() {
                    chosen = Some(PathBuf::from(expand_tilde(typed.trim())));
                }
            });
        if browse {
            self.pick_folder();
        }
        if let Some(p) = chosen {
            if p.is_dir() {
                self.set_folder(p);
            } else {
                self.notify(format!("Not a folder: {}", p.display()));
            }
        }
        if !open {
            self.show_welcome = false;
            self.settings.onboarded = true;
            self.mark_dirty();
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Search syntax & shortcuts").open(&mut self.show_help).default_width(520.0).show(ctx, |ui| {
            ui.label(RichText::new(SEARCH_HELP).monospace());
            ui.separator();
            ui.label(RichText::new("Channels").strong());
            for c in Channel::ALL {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(c.label()).color(self.palette.channel(c)).monospace());
                    ui.label(RichText::new(c.description()).small());
                });
            }
            ui.separator();
            ui.label(RichText::new("Keyboard").strong());
            ui.label(RichText::new(
                "Ctrl+F      focus the search box of the active pane\n\
                 Esc         clear search / selection\n\
                 PgUp/PgDn   scroll (mouse over a pane)\n\
                 Home/End    oldest / newest (End resumes following)\n\
                 Click       select a line, Shift+Click extends\n\
                 Ctrl+C      copy selected lines, Ctrl+A select all\n\
                 Ctrl+L      clear (hide current lines)\n\
                 F5          reload from disk\n\
                 Ctrl+,      settings\n\
                 Ctrl +/-    zoom",
            ).monospace());
            ui.separator();
            ui.label("Drag tabs to rearrange panes, split them side by side, or float them. Right-click a line for more.");
        });
        egui::Window::new("About UOC Journal")
            .open(&mut self.show_about)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.heading("UOC Journal");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A fast, searchable journal for ClassicUO-based Ultima Online clients.");
                ui.label(RichText::new(format!("Settings: {}", self.config_dir.display())).small());
            });
    }
}

impl eframe::App for JournalApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        self.theme_list_shown = false;
        self.theme_hovered = None;
        self.font_list_shown = false;
        self.font_hovered = None;
        self.shortcuts(&ctx);

        let pal = self.palette.clone();
        egui::Panel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(pal.panel)
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(pal.panel)
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ui, |ui| self.status_bar(ui));

        // Pane rendering.
        let style = RowStyle {
            palette: &pal,
            font: FontId::new(
                self.settings.font_size.clamp(8.0, 40.0),
                if self.journal_font {
                    FontFamily::Name("journal".into())
                } else if self.settings.monospace {
                    FontFamily::Monospace
                } else {
                    FontFamily::Proportional
                },
            ),
            time_pattern: self
                .settings
                .time_format
                .pattern(&self.settings.time_custom),
            badges: self.settings.show_badges,
            badge_text: Channel::ALL.map(|c| self.settings.badge(c)),
            color_names: self.settings.color_names,
            character_tags: self.settings.character_tags && self.characters.len() > 1,
            highlights: &self.highlights,
        };
        let mut tabs = Tabs {
            panes: &mut self.panes,
            cx: PaneCtx {
                store: &self.store,
                style: &style,
                characters: &self.characters,
                min_id: self.clear_mark,
            },
            actions: Vec::new(),
            closed: Vec::new(),
            add: Vec::new(),
        };
        let mut dock_style = egui_dock::Style::from_egui(ui.style());
        dock_style.tab_bar.bg_fill = pal.panel;
        dock_style.main_surface_border_stroke = egui::Stroke::NONE;
        dock_style.separator.color_idle = pal.border;
        dock_style.separator.color_hovered = pal.accent;
        dock_style.separator.width = 2.0;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.panel))
            .show(ui, |ui| {
                DockArea::new(&mut self.dock)
                    .style(dock_style)
                    .show_add_buttons(true)
                    .show_add_popup(true)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut tabs);
            });
        let Tabs {
            actions,
            closed,
            add,
            ..
        } = tabs;
        for id in closed {
            self.panes.remove(&id);
            self.mark_dirty();
        }
        for (template, path) in add {
            self.open_pane(template.make(0), Some(path));
        }
        for action in actions {
            match action {
                PaneAction::OpenPane(cfg) => self.open_pane(cfg, None),
                PaneAction::AddHighlight(word) => {
                    self.settings.highlights.push(HighlightRule {
                        pattern: word.clone(),
                        color: Rgb::from_color(self.palette.accent),
                        ..Default::default()
                    });
                    self.compile_highlights();
                    self.mark_dirty();
                    self.notify(format!(
                        "Highlighting “{word}” (edit in Settings → Highlights)"
                    ));
                }
                PaneAction::ConfigChanged => self.mark_dirty(),
            }
        }
        if self.panes.is_empty() {
            self.reset_layout();
        }

        self.welcome_window(&ctx);
        self.help_window(&ctx);
        crate::settings_ui::show(self, &ctx);

        // Live theme preview: hovering a theme shows it; leaving the list without
        // clicking (menu or dropdown closes) goes back to the saved theme.
        let want = if !self.theme_list_shown {
            None
        } else if self.theme_hovered.is_some() {
            self.theme_hovered.clone()
        } else {
            self.theme_preview.clone()
        };
        let want_font = if !self.font_list_shown {
            None
        } else if self.font_hovered.is_some() {
            self.font_hovered.clone()
        } else {
            self.font_preview.clone()
        };
        if want_font != self.font_preview {
            self.font_preview = want_font;
            self.apply_style(&ctx);
            ctx.request_repaint();
        }
        if want != self.theme_preview {
            self.theme_preview = want;
            self.apply_style(&ctx);
            ctx.request_repaint();
        }

        // Persist.
        if self.settings_dirty && self.settings_changed_at.elapsed() > Duration::from_millis(1200) {
            self.save_now();
        }
        if self.last_layout_check.elapsed() > Duration::from_secs(5) {
            self.last_layout_check = Instant::now();
            self.save_layout();
        }
        if self.settings_dirty {
            ctx.request_repaint_after(Duration::from_millis(1300));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_now();
        }
    }
}

impl Drop for JournalApp {
    fn drop(&mut self) {
        self.save_now();
    }
}

struct Tabs<'a> {
    panes: &'a mut BTreeMap<u64, Pane>,
    cx: PaneCtx<'a>,
    actions: Vec<PaneAction>,
    closed: Vec<u64>,
    add: Vec<(Template, NodePath)>,
}

impl TabViewer for Tabs<'_> {
    type Tab = u64;

    fn id(&mut self, tab: &mut u64) -> egui::Id {
        egui::Id::new(("pane", *tab))
    }

    fn title(&mut self, tab: &mut u64) -> egui::WidgetText {
        self.panes
            .get(tab)
            .map(|p| p.title())
            .unwrap_or_else(|| "?".into())
            .into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut u64) {
        if let Some(p) = self.panes.get_mut(tab) {
            let acts = p.ui(ui, &self.cx);
            self.actions.extend(acts);
        }
    }

    fn on_close(&mut self, tab: &mut u64) -> OnCloseResponse {
        self.closed.push(*tab);
        OnCloseResponse::Close
    }

    fn scroll_bars(&self, _tab: &u64) -> [bool; 2] {
        [false, false]
    }

    fn add_popup(&mut self, ui: &mut egui::Ui, path: NodePath) {
        ui.set_min_width(200.0);
        ui.label(RichText::new("New pane").strong());
        for t in Template::MENU {
            if ui.button(t.label()).clicked() {
                self.add.push((t, path));
                ui.close();
            }
        }
    }

    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut u64, _path: NodePath) {
        if let Some(p) = self.panes.get_mut(tab) {
            ui.label("Title");
            if ui.text_edit_singleline(&mut p.cfg.title).changed() {
                self.actions.push(PaneAction::ConfigChanged);
            }
            if ui.button("Duplicate").clicked() {
                let mut c = p.cfg.clone();
                c.title = format!("{} (copy)", c.title);
                self.actions.push(PaneAction::OpenPane(c));
                ui.close();
            }
        }
    }
}

pub fn watch_config(s: &Settings) -> WatchConfig {
    WatchConfig {
        folder: s.journal_folder.clone(),
        history_hours: s.history_hours,
        max_history_files: s.max_history_files,
        rules: s.rules.clone(),
        poll_ms: 40,
    }
}

fn default_panes(s: &mut Settings) -> Vec<PaneConfig> {
    let order = [
        Template::All,
        Template::System,
        Template::World,
        Template::GuildAlliance,
        Template::Chat,
        Template::Combat,
        Template::People,
        Template::Names,
        Template::Items,
    ];
    order.iter().map(|t| t.make(s.alloc_pane_id())).collect()
}

/// ```text
/// ┌──────────────────────┬──────────────────────┐
/// │ All | System | World │ Guild / Ally / Party │
/// │                      ├──────────────────────┤
/// │                      │ Chat                 │
/// ├───────────┬──────────┴──────────────────────┤
/// │ Combat    │ People | Name labels | Items    │
/// └───────────┴─────────────────────────────────┘
/// ```
fn default_layout(panes: &BTreeMap<u64, Pane>) -> DockState<u64> {
    let by_title = |kind: Template| -> Vec<u64> {
        let want = kind.make(0);
        panes
            .values()
            .filter(|p| p.cfg.title == want.title && p.cfg.kind == want.kind)
            .map(|p| p.cfg.id)
            .take(1)
            .collect()
    };
    let main: Vec<u64> = [Template::All, Template::System, Template::World]
        .into_iter()
        .flat_map(by_title)
        .collect();
    if main.is_empty() {
        return DockState::new(panes.keys().copied().collect());
    }
    let mut dock = DockState::new(main);
    let surface = dock.main_surface_mut();
    let gap = by_title(Template::GuildAlliance);
    let chat = by_title(Template::Chat);
    let combat = by_title(Template::Combat);
    let lower: Vec<u64> = [Template::People, Template::Names, Template::Items]
        .into_iter()
        .flat_map(by_title)
        .collect();
    let [top, bottom] = if !combat.is_empty() || !lower.is_empty() {
        let mut first = combat.clone();
        if first.is_empty() {
            first = lower.clone();
        }
        surface.split_below(NodeIndex::root(), 0.66, first)
    } else {
        [NodeIndex::root(), NodeIndex::root()]
    };
    if !combat.is_empty() && !lower.is_empty() {
        surface.split_right(bottom, 0.4, lower);
    }
    if !gap.is_empty() {
        let [_, right] = surface.split_right(top, 0.58, gap);
        if !chat.is_empty() {
            surface.split_below(right, 0.5, chat);
        }
    } else if !chat.is_empty() {
        surface.split_right(top, 0.58, chat);
    }
    // Any pane not placed yet goes into the first leaf.
    let placed: Vec<u64> = dock.iter_all_tabs().map(|(_, t)| *t).collect();
    for id in panes.keys() {
        if !placed.contains(id) {
            dock.push_to_first_leaf(*id);
        }
    }
    dock
}

fn load_layout(dir: &Path, panes: &BTreeMap<u64, Pane>) -> Option<DockState<u64>> {
    let text = std::fs::read_to_string(dir.join(config::LAYOUT_FILE)).ok()?;
    let mut dock: DockState<u64> = serde_json::from_str(&text).ok()?;
    dock.retain_tabs(|t| panes.contains_key(t));
    let placed: Vec<u64> = dock.iter_all_tabs().map(|(_, t)| *t).collect();
    if placed.is_empty() {
        return None;
    }
    for id in panes.keys() {
        if !placed.contains(id) {
            dock.push_to_first_leaf(*id);
        }
    }
    Some(dock)
}

/// `~/…/Data/Client/JournalLogs` – home abbreviated, long middles elided.
pub fn short_path(p: &Path) -> String {
    let mut s = p.display().to_string();
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().to_string();
        if !home.is_empty() && s.starts_with(&home) {
            s = format!("~{}", &s[home.len()..]);
        }
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > 72 {
        let head: String = chars[..24].iter().collect();
        let tail: String = chars[chars.len() - 45..].iter().collect();
        s = format!("{head}…{tail}");
    }
    s
}

/// A small filled circle (the default fonts have no ● glyph).
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    resp
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{rest}", home.to_string_lossy());
        }
    }
    p.to_string()
}
