//! Settings window: journal source, appearance & theme editor, highlights,
//! custom classification rules.

use std::path::PathBuf;

use eframe::egui::{self, RichText};
use uoj_core::classify::{compile_rules, Classifier, Ctx, UserRule};
use uoj_core::{parse, Channel, ChannelSet};

use crate::app::{expand_tilde, JournalApp};
use crate::config::{HighlightRule, TimeFormat};
use crate::theme::{Rgb, Theme};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Journal,
    Appearance,
    Characters,
    Channels,
    Highlights,
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub tab: Tab,
    pub folder_text: String,
    rules_draft: Option<Vec<UserRule>>,
    test_line: String,
    theme_import: String,
    new_theme_name: String,
}

pub fn show(app: &mut JournalApp, ctx: &egui::Context) {
    if !app.settings_ui.open {
        return;
    }
    let mut open = true;
    egui::Window::new("Settings")
        .open(&mut open)
        .default_width(620.0)
        .default_height(520.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let t = &mut app.settings_ui.tab;
                ui.selectable_value(t, Tab::Journal, "Journal");
                ui.selectable_value(t, Tab::Appearance, "Appearance");
                ui.selectable_value(t, Tab::Characters, "Characters");
                ui.selectable_value(t, Tab::Channels, "Channels & rules");
                ui.selectable_value(t, Tab::Highlights, "Highlights & alerts");
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match app.settings_ui.tab {
                    Tab::Journal => journal_tab(app, ui),
                    Tab::Appearance => appearance_tab(app, ui),
                    Tab::Characters => characters_tab(app, ui),
                    Tab::Channels => channels_tab(app, ui),
                    Tab::Highlights => highlights_tab(app, ui),
                });
        });
    if !open {
        app.settings_ui.open = false;
    }
}

fn journal_tab(app: &mut JournalApp, ui: &mut egui::Ui) {
    ui.label(RichText::new("Journal folder").strong());
    ui.label(
        RichText::new(
            "The client's JournalLogs folder (ClassicUO: Data/Client/JournalLogs). Every running client \
             writes its own file there; all of them are followed at once.",
        )
        .small(),
    );
    if app.settings_ui.folder_text.is_empty() {
        if let Some(f) = &app.settings.journal_folder {
            app.settings_ui.folder_text = f.display().to_string();
        }
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.settings_ui.folder_text).desired_width(360.0));
        if ui.button("Use").clicked() {
            let p = PathBuf::from(expand_tilde(app.settings_ui.folder_text.trim()));
            if p.is_dir() {
                app.set_folder(p);
            } else {
                app.notify(format!("Not a folder: {}", p.display()));
            }
        }
        if ui.button("Browse…").clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                app.settings_ui.folder_text = dir.display().to_string();
                app.set_folder(dir);
            }
        }
        if ui.button("Detect").clicked() {
            let ctx = ui.ctx().clone();
            app.start_detect(&ctx);
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new("History").strong());
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Load files from the last");
        changed |= ui
            .add(
                egui::DragValue::new(&mut app.settings.history_hours)
                    .range(0..=24 * 365)
                    .suffix(" h"),
            )
            .changed();
        ui.label("but at most");
        changed |= ui
            .add(egui::DragValue::new(&mut app.settings.max_history_files).range(1..=5000))
            .changed();
        ui.label("files.");
    });
    ui.label(
        RichText::new("The newest file is always followed. Journal → “Load the whole folder's history” loads everything once.")
            .small(),
    );
    if changed {
        app.mark_dirty();
    }
    if ui.button("Apply & reload").clicked() {
        app.reconfigure_watcher();
        app.watcher.send(uoj_core::watcher::Command::Reload);
    }
    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("Files").strong());
    for f in app.status.files.iter().rev().take(50) {
        ui.horizontal(|ui| {
            crate::app::status_dot(
                ui,
                if f.active {
                    app.palette.heal
                } else {
                    app.palette.dim
                },
            );
            ui.label(RichText::new(&f.name).monospace().small());
            if let Some(c) = &f.character {
                ui.label(RichText::new(c).color(app.palette.accent).small());
            }
            ui.label(
                RichText::new(format!("{:.1} KB", f.bytes as f64 / 1024.0))
                    .small()
                    .weak(),
            );
        });
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!(
            "Settings are stored in {}",
            app.config_dir.display()
        ))
        .small()
        .weak(),
    );
}

fn appearance_tab(app: &mut JournalApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut restyle = false;
    let mut dirty = false;

    egui::Grid::new("appearance")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Theme");
            egui::ComboBox::from_id_salt("theme_pick")
                .selected_text(app.settings.theme.clone())
                .height(420.0)
                .show_ui(ui, |ui| {
                    app.theme_list_shown = true;
                    for t in app.settings.all_themes() {
                        let r = ui.selectable_label(t.name == app.settings.theme, &t.name);
                        if r.hovered() {
                            app.theme_hovered = Some(t.name.clone());
                        }
                        if r.clicked() {
                            app.settings.theme = t.name.clone();
                            app.theme_preview = None;
                            restyle = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("Text size");
            restyle |= ui
                .add(egui::Slider::new(&mut app.settings.font_size, 9.0..=28.0).step_by(0.5))
                .changed();
            ui.end_row();

            ui.label("Font file");
            ui.horizontal(|ui| {
                let label = app
                    .settings
                    .font_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "built-in".into());
                ui.label(RichText::new(label).small());
                if ui.button("Choose…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Fonts", &["ttf", "otf"])
                        .pick_file()
                    {
                        app.settings.font_path = Some(p);
                        restyle = true;
                    }
                }
                if app.settings.font_path.is_some() && ui.button("Reset").clicked() {
                    app.settings.font_path = None;
                    restyle = true;
                }
            });
            ui.end_row();

            ui.label("Journal text");
            dirty |= ui
                .checkbox(&mut app.settings.monospace, "Monospace")
                .changed();
            ui.end_row();

            ui.label("Time column");
            ui.horizontal(|ui| {
                dirty |= ui
                    .radio_value(&mut app.settings.time_format, TimeFormat::Time, "HH:MM")
                    .changed();
                dirty |= ui
                    .radio_value(
                        &mut app.settings.time_format,
                        TimeFormat::DateTime,
                        "MM-DD HH:MM",
                    )
                    .changed();
                dirty |= ui
                    .radio_value(&mut app.settings.time_format, TimeFormat::Hidden, "Hidden")
                    .changed();
            });
            ui.end_row();

            ui.label("Lines");
            ui.vertical(|ui| {
                dirty |= ui
                    .checkbox(
                        &mut app.settings.show_badges,
                        "Channel badges (GLD, ALY, SYS …)",
                    )
                    .changed();
                dirty |= ui
                    .checkbox(
                        &mut app.settings.color_names,
                        "Give every name its own colour",
                    )
                    .changed();
                dirty |= ui
                    .checkbox(
                        &mut app.settings.character_tags,
                        "Prefix lines with your character when several clients run",
                    )
                    .changed();
            });
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("Theme colours").strong());
    ui.label(RichText::new("Editing a built-in theme creates a custom copy.").small());

    let mut theme = app.settings.current_theme();
    let before = theme.clone();
    egui::Grid::new("theme_colors")
        .num_columns(4)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            let cell = |ui: &mut egui::Ui, label: &str, c: &mut Rgb, n: &mut usize| {
                ui.label(label);
                ui.color_edit_button_srgb(&mut c.0);
                *n += 1;
                if n.is_multiple_of(2) {
                    ui.end_row();
                }
            };
            let mut n = 0;
            cell(ui, "Background", &mut theme.background, &mut n);
            cell(ui, "Panels", &mut theme.panel, &mut n);
            cell(ui, "Controls", &mut theme.surface, &mut n);
            cell(ui, "Borders", &mut theme.border, &mut n);
            cell(ui, "Text", &mut theme.text, &mut n);
            cell(ui, "Dim text / time", &mut theme.dim, &mut n);
            cell(ui, "Accent", &mut theme.accent, &mut n);
            cell(ui, "Selection", &mut theme.selection, &mut n);
            cell(ui, "Search match", &mut theme.search_match, &mut n);
            cell(ui, "Mention tint", &mut theme.mention, &mut n);
            cell(ui, "Damage taken", &mut theme.damage_taken, &mut n);
            cell(ui, "Damage dealt", &mut theme.damage_dealt, &mut n);
            cell(ui, "Healing", &mut theme.heal, &mut n);
            cell(ui, "Names", &mut theme.name_color, &mut n);
            if !n.is_multiple_of(2) {
                ui.end_row();
            }
        });
    ui.horizontal(|ui| {
        ui.label("Dark theme");
        ui.checkbox(&mut theme.dark, "");
    });
    ui.label(
        RichText::new("Channel colours and row backgrounds are on the “Channels & rules” page.")
            .small(),
    );
    if commit_theme(app, &before, theme.clone()) {
        theme = app.settings.current_theme();
        restyle = true;
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.settings_ui.new_theme_name)
                .hint_text("new theme name")
                .desired_width(160.0),
        );
        if ui.button("Save as").clicked() && !app.settings_ui.new_theme_name.trim().is_empty() {
            let mut t = theme.clone();
            t.name = app.settings_ui.new_theme_name.trim().to_string();
            app.settings.custom_themes.retain(|x| x.name != t.name);
            app.settings.theme = t.name.clone();
            app.settings.custom_themes.push(t);
            app.settings_ui.new_theme_name.clear();
            restyle = true;
        }
        let custom = app
            .settings
            .custom_themes
            .iter()
            .any(|t| t.name == theme.name);
        if custom && ui.button("Delete this theme").clicked() {
            app.settings.custom_themes.retain(|t| t.name != theme.name);
            app.settings.theme = Theme::default().name;
            restyle = true;
        }
        if ui.button("Copy as TOML").clicked() {
            if let Ok(s) = toml::to_string_pretty(&theme) {
                ui.ctx().copy_text(s);
            }
        }
    });
    ui.collapsing("Import a theme (paste TOML)", |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut app.settings_ui.theme_import)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        if ui.button("Import").clicked() {
            match toml::from_str::<Theme>(&app.settings_ui.theme_import) {
                Ok(t) => {
                    app.settings.custom_themes.retain(|x| x.name != t.name);
                    app.settings.theme = t.name.clone();
                    app.settings.custom_themes.push(t);
                    app.settings_ui.theme_import.clear();
                    restyle = true;
                }
                Err(e) => app.notify(format!("Theme import failed: {e}")),
            }
        }
    });

    if restyle {
        app.apply_style(&ctx);
        dirty = true;
    }
    if dirty {
        app.mark_dirty();
    }
}

fn channel_set_menu(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    set: &mut ChannelSet,
    empty_label: &str,
) -> bool {
    let mut changed = false;
    let text = if set.is_empty() {
        empty_label.to_string()
    } else if set.len() <= 2 {
        set.iter().map(|c| c.label()).collect::<Vec<_>>().join(", ")
    } else {
        format!("{} channels", set.len())
    };
    ui.push_id(id, |ui| {
        let cfg = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        egui::containers::menu::MenuButton::new(text)
            .config(cfg)
            .ui(ui, |ui| {
                if ui.button(empty_label).clicked() {
                    *set = ChannelSet::EMPTY;
                    changed = true;
                }
                for c in Channel::ALL {
                    let mut on = set.contains(c);
                    if ui.checkbox(&mut on, c.label()).changed() {
                        set.set(c, on);
                        changed = true;
                    }
                }
            });
    });
    changed
}

fn highlights_tab(app: &mut JournalApp, ui: &mut egui::Ui) {
    ui.label(
        "Colour words or whole lines that match. With “alert”, the window asks for attention \
         (taskbar flash / urgency hint) when a new line matches while it is in the background.",
    );
    ui.label(
        RichText::new(
            "Patterns: words separated by | (case-insensitive), or a regular expression.",
        )
        .small(),
    );
    ui.add_space(6.0);
    let mut changed = false;
    let mut remove = None;
    for (i, h) in app.settings.highlights.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                changed |= ui
                    .checkbox(&mut h.enabled, "")
                    .on_hover_text("enabled")
                    .changed();
                changed |= ui.color_edit_button_srgb(&mut h.color.0).changed();
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut h.pattern)
                            .desired_width(200.0)
                            .hint_text("reds|pk"),
                    )
                    .changed();
                changed |= ui.checkbox(&mut h.regex, "regex").changed();
                changed |= ui.checkbox(&mut h.whole_line, "whole line").changed();
                changed |= ui.checkbox(&mut h.alert, "alert").changed();
                changed |= channel_set_menu(ui, ("hl", i), &mut h.channels, "All channels");
                if ui.small_button("🗑").on_hover_text("remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        app.settings.highlights.remove(i);
        changed = true;
    }
    if ui.button("+ Add highlight").clicked() {
        app.settings.highlights.push(HighlightRule::default());
        changed = true;
    }
    for e in &app.highlight_errors {
        ui.colored_label(app.palette.damage_taken, e);
    }
    if changed {
        app.compile_highlights();
        app.mark_dirty();
    }
}

/// Apply an edited copy of the current theme; editing a built-in theme saves a
/// custom copy instead. Returns true if anything changed.
fn commit_theme(app: &mut JournalApp, before: &Theme, mut theme: Theme) -> bool {
    if theme == *before {
        return false;
    }
    if crate::theme::presets().iter().any(|p| p.name == theme.name) {
        theme.name = format!("{} (custom)", theme.name);
    }
    app.settings.custom_themes.retain(|t| t.name != theme.name);
    app.settings.theme = theme.name.clone();
    app.settings.custom_themes.push(theme);
    true
}

fn rule_editor(ui: &mut egui::Ui, r: &mut UserRule, key: usize) -> bool {
    let mut remove = false;
    ui.push_id(("rule", key), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut r.enabled, "").on_hover_text("enabled");
            ui.label("speaker");
            ui.add(
                egui::TextEdit::singleline(&mut r.speaker)
                    .desired_width(100.0)
                    .hint_text("any"),
            );
            ui.label("from");
            channel_set_menu(ui, ("from", key), &mut r.from, "any channel");
            ui.label("text matches");
            ui.add(
                egui::TextEdit::singleline(&mut r.pattern)
                    .desired_width(180.0)
                    .hint_text("regex, e.g. (?i)revealed"),
            );
            egui::ComboBox::from_id_salt(("to", key))
                .selected_text(format!("→ {}", r.to.label()))
                .show_ui(ui, |ui| {
                    for c in Channel::ALL {
                        ui.selectable_value(&mut r.to, c, c.label());
                    }
                });
            if ui.small_button("🗑").on_hover_text("remove rule").clicked() {
                remove = true;
            }
        });
        if !r.pattern.is_empty() {
            if let Err(e) = regex::Regex::new(&r.pattern) {
                ui.colored_label(egui::Color32::from_rgb(230, 80, 80), e.to_string());
            }
        }
    });
    remove
}

fn channels_tab(app: &mut JournalApp, ui: &mut egui::Ui) {
    ui.label(
        "Every line lands in one channel. Here you can colour each channel, give its lines a \
         background, and add regular-expression rules that move matching lines into it. Rules \
         run after the built-in sorting, in list order, and the first match wins.",
    );
    ui.add_space(6.0);
    let mut draft = app
        .settings_ui
        .rules_draft
        .take()
        .unwrap_or_else(|| app.settings.rules.clone());
    let rules_changed = draft != app.settings.rules;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                rules_changed,
                egui::Button::new("Apply rules (re-reads the journal)"),
            )
            .clicked()
        {
            app.settings.rules = draft.clone();
            app.reconfigure_watcher();
            app.mark_dirty();
        }
        if ui
            .add_enabled(rules_changed, egui::Button::new("Revert"))
            .clicked()
        {
            draft = app.settings.rules.clone();
        }
        if rules_changed {
            ui.label(
                RichText::new("unapplied rule changes")
                    .color(app.palette.accent)
                    .small(),
            );
        }
    });
    ui.add_space(6.0);

    let mut theme = app.settings.current_theme();
    let before = theme.clone();
    let mut remove: Option<usize> = None;
    for c in Channel::ALL {
        let label = c.label().to_string();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let fg = theme.channels.get(&label).copied().unwrap_or(theme.text);
                let bg = theme.channel_backgrounds.get(&label).copied();
                let mut sample = RichText::new(format!(" {label} "))
                    .color(fg.color())
                    .strong();
                sample = sample.background_color(bg.unwrap_or(theme.background).color());
                ui.add_sized([90.0, 20.0], egui::Label::new(sample));
                ui.label("text");
                let mut fg_edit = fg;
                if ui.color_edit_button_srgb(&mut fg_edit.0).changed() {
                    theme.channels.insert(label.clone(), fg_edit);
                }
                let mut has_bg = bg.is_some();
                if ui.checkbox(&mut has_bg, "background").changed() {
                    if has_bg {
                        let tint = crate::theme::default_background(&theme, fg);
                        theme.channel_backgrounds.insert(label.clone(), tint);
                    } else {
                        theme.channel_backgrounds.remove(&label);
                    }
                }
                if let Some(mut b) = theme.channel_backgrounds.get(&label).copied() {
                    if ui.color_edit_button_srgb(&mut b.0).changed() {
                        theme.channel_backgrounds.insert(label.clone(), b);
                    }
                }
                ui.label(RichText::new(c.description()).small().weak());
            });
            let mine: Vec<usize> = (0..draft.len()).filter(|&i| draft[i].to == c).collect();
            let header = if mine.is_empty() {
                "Rules: none".to_string()
            } else {
                format!("Rules: {}", mine.len())
            };
            egui::CollapsingHeader::new(header)
                .id_salt(("rules", c.index()))
                .default_open(!mine.is_empty())
                .show(ui, |ui| {
                    for i in mine {
                        if rule_editor(ui, &mut draft[i], i) {
                            remove = Some(i);
                        }
                    }
                    if ui
                        .small_button(format!("+ Add rule into {label}"))
                        .clicked()
                    {
                        draft.push(UserRule {
                            name: format!("{label} rule"),
                            to: c,
                            ..Default::default()
                        });
                    }
                });
        });
    }
    if let Some(i) = remove {
        draft.remove(i);
    }
    if commit_theme(app, &before, theme) {
        let ctx = ui.ctx().clone();
        app.apply_style(&ctx);
        app.mark_dirty();
    }

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("Try it").strong());
    ui.label(RichText::new("Paste a journal line (with or without the [time] prefix):").small());
    ui.add(
        egui::TextEdit::singleline(&mut app.settings_ui.test_line)
            .desired_width(f32::INFINITY)
            .hint_text(
                "[03/14/2026 18:12]  [Alliance][Lysa Quill]: [OAK] reds near the north bridge",
            ),
    );
    let line = app.settings_ui.test_line.trim();
    if !line.is_empty() {
        let (name, text) = match parse::split_line(line) {
            Some(r) => (r.name, r.text),
            None => parse::split_name(line),
        };
        let cl = Classifier::new();
        let no = |_: &str| false;
        let c = cl.classify(
            name,
            text,
            &Ctx {
                self_name: None,
                staff_body: false,
                is_npc: &no,
                is_pet: &no,
            },
        );
        let mut channel = c.channel;
        let mut by_rule: Option<usize> = None;
        let (rules, _) = compile_rules(&draft);
        if c.label.is_none() {
            for (i, r) in rules.iter().enumerate() {
                if let Some(ch) = r.apply(c.speaker, text, channel) {
                    channel = ch;
                    by_rule = Some(i);
                    break;
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label("speaker:");
            ui.label(RichText::new(c.speaker).strong());
            ui.label("channel:");
            ui.label(
                RichText::new(channel.label())
                    .color(app.palette.channel(channel))
                    .strong(),
            );
            match by_rule {
                Some(_) => ui.label(RichText::new("(moved by a custom rule)").small()),
                None => {
                    ui.label(RichText::new(format!("(built-in: {})", c.channel.label())).small())
                }
            };
        });
    }
    app.settings_ui.rules_draft = Some(draft);
}

fn characters_tab(app: &mut JournalApp, ui: &mut egui::Ui) {
    ui.label(
        "When more than one client is running, each line is tagged with a chip for the character \
         whose journal it came from. New characters get initials and a colour automatically; \
         change them here (up to three letters).",
    );
    let mut changed = ui
        .checkbox(
            &mut app.settings.character_tags,
            "Show character chips (with two or more characters)",
        )
        .changed();
    ui.add_space(6.0);
    app.ensure_chips();
    if app.settings.chips.is_empty() {
        ui.label(
            RichText::new(
                "No characters seen yet. Log in with journal saving on and they appear here.",
            )
            .weak(),
        );
    }
    let seen = app.store.characters();
    let mut remove: Option<usize> = None;
    egui::Grid::new("chips")
        .num_columns(3)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Full Name").strong());
            ui.label(RichText::new("Initials").strong());
            ui.label(RichText::new("Color").strong());
            ui.end_row();
            for i in 0..app.settings.chips.len() {
                let name = app.settings.chips[i].name.clone();
                ui.horizontal(|ui| {
                    crate::logview::character_pill(ui, &app.palette, &name);
                    ui.label(&name);
                    if !seen.contains(&name)
                        && ui
                            .small_button("forget")
                            .on_hover_text("not in the loaded journals")
                            .clicked()
                    {
                        remove = Some(i);
                    }
                });
                let chip = &mut app.settings.chips[i];
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut chip.label)
                            .char_limit(3)
                            .desired_width(48.0)
                            .hint_text(crate::logview::initials(&name)),
                    )
                    .changed();
                let color = chip.color.get_or_insert(Rgb::hex(0xffd166));
                changed |= ui.color_edit_button_srgb(&mut color.0).changed();
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        app.settings.chips.remove(i);
        changed = true;
    }
    if changed {
        app.refresh_chips();
        app.mark_dirty();
    }
}
