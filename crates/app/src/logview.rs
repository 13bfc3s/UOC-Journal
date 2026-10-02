//! Virtualised, wrapped journal view.
//!
//! Only rows that are on screen are laid out, so the cost per frame does not
//! depend on how many lines a pane holds. Scroll position is kept as "row index
//! of the top line + pixels of it hidden above the viewport", which makes
//! variable row heights (wrapped text) exact without measuring every row, keeps
//! the view stable when lines are appended, and makes "follow the tail" trivial.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, text::LayoutJob, Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Key, Pos2,
    Rect, Sense, Stroke, TextFormat, Vec2,
};
use regex::Regex;
use uoj_core::classify::combat_number;
use uoj_core::{flags, time, Channel, ChannelSet, Entry, Query, Store};

use crate::config::HighlightRule;
use crate::theme::{blend, Palette};

/// A highlight rule ready for matching.
pub struct Highlight {
    pub re: Regex,
    pub color: Color32,
    pub whole_line: bool,
    pub alert: bool,
    pub channels: ChannelSet,
}

impl Highlight {
    pub fn compile(rule: &HighlightRule) -> Result<Highlight, String> {
        let pat = if rule.regex {
            rule.pattern.clone()
        } else {
            rule.pattern
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join("|")
        };
        if pat.trim().is_empty() {
            return Err("empty pattern".into());
        }
        let re = Regex::new(&format!("(?i){pat}")).map_err(|e| e.to_string())?;
        Ok(Highlight {
            re,
            color: rule.color.color(),
            whole_line: rule.whole_line,
            alert: rule.alert,
            channels: rule.channels,
        })
    }

    pub fn applies(&self, channel: Channel) -> bool {
        self.channels.is_empty() || self.channels.contains(channel)
    }
}

/// Display options shared by every pane.
pub struct RowStyle<'a> {
    pub palette: &'a Palette,
    pub font: FontId,
    /// strftime-style time column pattern; `None` hides the column.
    pub time_pattern: Option<String>,
    pub badges: bool,
    pub color_names: bool,
    /// Prefix rows with the character name.
    pub character_tags: bool,
    pub highlights: &'a [Highlight],
}

/// Scroll position, selection and other per-pane view state.
#[derive(Debug, Clone)]
pub struct LogState {
    pub follow: bool,
    pub top: usize,
    pub offset: f32,
    /// Entry id at `top`, used to re-anchor after the row list is rebuilt.
    pub top_id: Option<u32>,
    pub sel_anchor: Option<u32>,
    pub sel_cursor: Option<u32>,
    /// Rows appended while scrolled away from the bottom.
    pub unseen: usize,
    context_row: Option<u32>,
    visible_rows: usize,
    thumb_grab: Option<f32>,
}

impl Default for LogState {
    fn default() -> Self {
        LogState {
            follow: true,
            top: 0,
            offset: 0.0,
            top_id: None,
            sel_anchor: None,
            sel_cursor: None,
            unseen: 0,
            context_row: None,
            visible_rows: 0,
            thumb_grab: None,
        }
    }
}

impl LogState {
    /// Call after the view's row list was rebuilt (filter change, reclassification).
    pub fn reanchor(&mut self, rows: &[u32]) {
        if self.follow {
            return;
        }
        match self.top_id {
            Some(id) => {
                let idx = rows.partition_point(|&r| r < id);
                if rows.get(idx) != Some(&id) {
                    self.offset = 0.0;
                }
                self.top = idx.min(rows.len().saturating_sub(1));
            }
            None => self.top = 0,
        }
    }

    pub fn rows_appended(&mut self, n: usize) {
        if !self.follow {
            self.unseen += n;
        }
    }

    pub fn jump_to_end(&mut self) {
        self.follow = true;
        self.unseen = 0;
    }

    fn selection(&self) -> Option<(u32, u32)> {
        match (self.sel_anchor, self.sel_cursor) {
            (Some(a), Some(c)) => Some((a.min(c), a.max(c))),
            _ => None,
        }
    }
}

/// Something the pane or app should do in response to the user.
#[derive(Debug, Clone, PartialEq)]
pub enum LogAction {
    /// Put `from:"name"` into this pane's search box.
    SearchSpeaker(String),
    /// Exclude a speaker in this pane's search box.
    ExcludeSpeaker(String),
    /// Open a new pane showing only this speaker.
    OpenSpeakerPane(String),
    /// Add a highlight rule for a word.
    Highlight(String),
}

/// Render one entry as `[HH:MM] speaker: text` for the clipboard.
/// Full timestamp, with seconds when they are known.
pub fn full_stamp(e: &Entry) -> String {
    match e.seconds() {
        Some(s) => time::format(e.time, Some(s), "%Y-%m-%d %H:%M:%S"),
        None => time::ymd_hm(e.time),
    }
}

pub fn plain_line(store: &Store, e: &Entry) -> String {
    let speaker = store.speaker(e);
    if speaker.is_empty() {
        format!("[{}] {}", full_stamp(e), store.text(e))
    } else {
        format!("[{}] {}: {}", full_stamp(e), speaker, store.text(e))
    }
}

struct Laid {
    id: u32,
    galley: Arc<Galley>,
    height: f32,
}

pub struct LogView<'a> {
    pub store: &'a Store,
    pub rows: &'a [u32],
    pub style: &'a RowStyle<'a>,
    /// Queries whose matches get a highlighted background.
    pub queries: [&'a Query; 2],
    pub id: egui::Id,
}

const ROW_PAD: f32 = 2.0;
const SCROLLBAR_W: f32 = 10.0;
const LEFT_PAD: f32 = 6.0;

impl LogView<'_> {
    /// Width of the widest time stamp the pattern can produce.
    fn time_width(&self, ui: &egui::Ui) -> f32 {
        let Some(pat) = &self.style.time_pattern else {
            return 0.0;
        };
        // A late, long date: September, Wednesday, 23:58:58.
        let sample = time::minutes(2026, 9, 30, 23, 58).unwrap_or(0);
        text_width(ui, &time::format(sample, Some(58), pat), &self.style.font)
    }

    fn gutter_width(&self, ui: &egui::Ui) -> f32 {
        let mut w = 0.0;
        if self.style.time_pattern.is_some() {
            w += self.time_width(ui) + 8.0;
        }
        if self.style.badges {
            let badge_font = FontId::new(self.style.font.size * 0.72, FontFamily::Monospace);
            w += text_width(ui, "WWW", &badge_font) + 10.0;
        }
        if self.style.character_tags {
            w += pill_width(ui, self.style.font.size) + 6.0;
        }
        w
    }

    fn job(&self, e: &Entry, wrap: f32) -> LayoutJob {
        let st = self.style;
        let pal = st.palette;
        let store = self.store;
        let text = store.text(e);
        let lower = store.lower(e);
        let speaker = store.speaker(e);
        let dup = e.has(flags::DUP);
        let fade = |c: Color32| if dup { c.gamma_multiply(0.45) } else { c };
        let font = st.font.clone();
        let fmt = |color: Color32| TextFormat {
            font_id: font.clone(),
            color: fade(color),
            ..Default::default()
        };

        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap.max(40.0);

        let channel_color = pal.channel(e.channel);
        let name_color = if st.color_names {
            pal.name_color(speaker)
        } else {
            pal.name
        };
        let mut body_color = channel_color;
        let mut italics = false;

        match e.channel {
            Channel::Guild
            | Channel::Alliance
            | Channel::Party
            | Channel::Speech
            | Channel::Npc
            | Channel::Spell => {
                if !speaker.is_empty() {
                    job.append(speaker, 0.0, fmt(name_color));
                    job.append(": ", 0.0, fmt(pal.dim));
                }
            }
            Channel::Emote => {
                if !speaker.is_empty() {
                    job.append(speaker, 0.0, fmt(name_color));
                    job.append(" ", 0.0, fmt(pal.dim));
                }
                italics = true;
            }
            Channel::Combat => {
                if let Some(n) = combat_number(text) {
                    let who = if speaker == "System" { "you" } else { speaker };
                    job.append(
                        who,
                        0.0,
                        fmt(if st.color_names && who != "you" {
                            name_color
                        } else {
                            pal.text
                        }),
                    );
                    job.append("  ", 0.0, fmt(pal.dim));
                    body_color = if n > 0 {
                        pal.heal
                    } else if e.has(flags::INCOMING) {
                        pal.damage_taken
                    } else {
                        pal.damage_dealt
                    };
                } else if speaker != "System" && !speaker.is_empty() {
                    job.append(speaker, 0.0, fmt(name_color));
                    job.append(": ", 0.0, fmt(pal.dim));
                }
                if text.starts_with('*') {
                    italics = true;
                }
            }
            Channel::Names => {
                if text != speaker && !text.contains(speaker) {
                    job.append(speaker, 0.0, fmt(name_color));
                    job.append(" · ", 0.0, fmt(pal.dim));
                } else {
                    body_color = if st.color_names {
                        name_color
                    } else {
                        channel_color
                    };
                }
            }
            Channel::Items => {
                if !speaker.is_empty() && speaker != "You see" {
                    job.append(speaker, 0.0, fmt(pal.dim));
                    job.append(": ", 0.0, fmt(pal.dim));
                }
            }
            Channel::System | Channel::World | Channel::Skill | Channel::Client => {
                if speaker != "System" && !speaker.is_empty() {
                    job.append(speaker, 0.0, fmt(pal.dim));
                    job.append(": ", 0.0, fmt(pal.dim));
                }
            }
            Channel::Razor => {}
        }

        // Alliance chat starts with the sender's guild tag: dim it.
        let mut body_start = 0;
        if e.channel == Channel::Alliance && text.starts_with('[') {
            if let Some(close) = text.find("] ") {
                if close <= 10 {
                    job.append(&text[..close + 2], 0.0, fmt(pal.dim));
                    body_start = close + 2;
                }
            }
        }
        let body = &text[body_start..];
        let body_lower = &lower[body_start..];

        // Collect coloured word ranges and search-match ranges within the body.
        let mut search: Vec<Range<usize>> = Vec::new();
        for q in self.queries {
            q.highlights(body, body_lower, &mut search);
        }
        uoj_core::query::merge_ranges(&mut search);
        let mut words: Vec<(Range<usize>, Color32)> = Vec::new();
        for h in st
            .highlights
            .iter()
            .filter(|h| !h.whole_line && h.applies(e.channel))
        {
            for m in h.re.find_iter(body).take(32) {
                if !m.is_empty() {
                    words.push((m.range(), h.color));
                }
            }
        }

        let base = TextFormat {
            font_id: st.font.clone(),
            color: fade(body_color),
            italics,
            ..Default::default()
        };
        if search.is_empty() && words.is_empty() {
            job.append(body, 0.0, base);
            return job;
        }
        let mut cuts: Vec<usize> = vec![0, body.len()];
        for r in &search {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        for (r, _) in &words {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        cuts.retain(|&c| c <= body.len() && body.is_char_boundary(c));
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a == b {
                continue;
            }
            let mut f = base.clone();
            if let Some((_, c)) = words.iter().find(|(r, _)| r.start <= a && b <= r.end) {
                f.color = fade(*c);
                f.underline = Stroke::new(1.0, c.gamma_multiply(0.5));
            }
            if search.iter().any(|r| r.start <= a && b <= r.end) {
                f.background = pal.search_match;
            }
            job.append(&body[a..b], 0.0, f);
        }
        job
    }

    fn layout(&self, ui: &egui::Ui, idx: usize, wrap: f32) -> Laid {
        let id = self.rows[idx];
        let e = self.store.entry(id);
        let galley = ui.painter().layout_job(self.job(e, wrap));
        let height = galley.size().y.max(self.style.font.size) + ROW_PAD * 2.0;
        Laid { id, galley, height }
    }

    fn row_height(&self, ui: &egui::Ui, idx: usize, wrap: f32) -> f32 {
        self.layout(ui, idx, wrap).height
    }

    /// Draw the view into the remaining space of `ui`.
    pub fn show(&self, ui: &mut egui::Ui, state: &mut LogState) -> Vec<LogAction> {
        let mut actions = Vec::new();
        let pal = self.style.palette;
        let outer = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(outer, Sense::click());
        // A pane squeezed to almost nothing: draw nothing rather than do layout
        // maths with negative sizes.
        if outer.width() < SCROLLBAR_W + 24.0 || outer.height() < 6.0 {
            return actions;
        }
        let sb_rect = Rect::from_min_max(
            Pos2::new(outer.right() - SCROLLBAR_W, outer.top()),
            outer.right_bottom(),
        );
        let content = Rect::from_min_max(
            outer.left_top(),
            Pos2::new(sb_rect.left() - 2.0, outer.bottom()),
        );
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, CornerRadius::ZERO, pal.background);

        let n = self.rows.len();
        let gutter = self.gutter_width(ui);
        let wrap = (content.width() - gutter - LEFT_PAD * 2.0).max(60.0);

        // Clamp state to the current row count.
        if n == 0 {
            state.top = 0;
            state.offset = 0.0;
        } else if state.top >= n {
            state.top = n - 1;
            state.offset = 0.0;
        }

        // ---- input ------------------------------------------------------------
        let hovered = ui.rect_contains_pointer(outer);
        let typing = ui.ctx().egui_wants_keyboard_input();
        let mut scroll = 0.0f32;
        if hovered {
            scroll += ui.input(|i| i.smooth_scroll_delta.y);
        }
        if hovered && !typing {
            let page = content.height() * 0.9;
            ui.input(|i| {
                if i.key_pressed(Key::PageUp) {
                    scroll += page;
                }
                if i.key_pressed(Key::PageDown) {
                    scroll -= page;
                }
                if i.key_pressed(Key::ArrowUp) {
                    scroll += self.style.font.size * 1.5;
                }
                if i.key_pressed(Key::ArrowDown) {
                    scroll -= self.style.font.size * 1.5;
                }
            });
            if ui.input(|i| i.key_pressed(Key::Home)) && n > 0 {
                state.follow = false;
                state.top = 0;
                state.offset = 0.0;
            }
            if ui.input(|i| i.key_pressed(Key::End)) {
                state.jump_to_end();
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                state.sel_anchor = None;
                state.sel_cursor = None;
            }
            if ui.input(|i| i.modifiers.command && i.key_pressed(Key::A)) && n > 0 {
                state.sel_anchor = Some(self.rows[0]);
                state.sel_cursor = Some(self.rows[n - 1]);
            }
            let copy = ui.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                    || (i.modifiers.command && i.key_pressed(Key::C))
            });
            if copy {
                if let Some(text) = self.selection_text(state) {
                    ui.ctx().copy_text(text);
                }
            }
        }

        if scroll != 0.0 && n > 0 {
            state.follow = false;
            state.offset -= scroll;
            while state.offset < 0.0 {
                if state.top == 0 {
                    state.offset = 0.0;
                    break;
                }
                state.top -= 1;
                state.offset += self.row_height(ui, state.top, wrap);
            }
            loop {
                let h = self.row_height(ui, state.top, wrap);
                if state.offset >= h && state.top + 1 < n {
                    state.offset -= h;
                    state.top += 1;
                } else {
                    break;
                }
            }
        }

        // ---- layout visible rows ----------------------------------------------
        let mut laid: Vec<(f32, Laid)> = Vec::new();
        if n > 0 {
            if !state.follow {
                let mut y = content.top() - state.offset;
                let mut i = state.top;
                while i < n && y < content.bottom() {
                    let l = self.layout(ui, i, wrap);
                    let h = l.height;
                    laid.push((y, l));
                    y += h;
                    i += 1;
                }
                // Reached the end with room to spare: snap to the bottom and follow.
                if i >= n && y <= content.bottom() + 0.5 {
                    state.follow = true;
                }
            }
            if state.follow {
                laid.clear();
                let mut y = content.bottom();
                let mut i = n;
                while i > 0 && y > content.top() {
                    i -= 1;
                    let l = self.layout(ui, i, wrap);
                    y -= l.height;
                    laid.push((y, l));
                }
                laid.reverse();
                // Not enough rows to fill the view: draw them from the top.
                if y > content.top() {
                    let shift = y - content.top();
                    for (ly, _) in &mut laid {
                        *ly -= shift;
                    }
                    state.top = 0;
                    state.offset = 0.0;
                } else {
                    state.top = i;
                    state.offset = content.top() - y;
                }
                state.unseen = 0;
            }
        }
        state.top_id = self.rows.get(state.top).copied();
        state.visible_rows = laid.len();

        // ---- pointer interaction on rows ----------------------------------------
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let row_at = |p: Pos2| -> Option<u32> {
            if !content.contains(p) {
                return None;
            }
            laid.iter()
                .find(|(y, l)| p.y >= *y && p.y < *y + l.height)
                .map(|(_, l)| l.id)
        };
        if response.clicked() {
            if let Some(id) = pointer.and_then(row_at) {
                let shift = ui.input(|i| i.modifiers.shift);
                if shift && state.sel_anchor.is_some() {
                    state.sel_cursor = Some(id);
                } else if state.sel_anchor == Some(id) && state.sel_cursor == Some(id) {
                    state.sel_anchor = None;
                    state.sel_cursor = None;
                } else {
                    state.sel_anchor = Some(id);
                    state.sel_cursor = Some(id);
                }
            }
        }
        if response.secondary_clicked() {
            state.context_row = pointer.and_then(row_at);
        }
        let hover_row = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|_| hovered)
            .and_then(row_at);

        // ---- paint ---------------------------------------------------------------
        let clip = painter.with_clip_rect(content);
        let sel = state.selection();
        let badge_font = FontId::new(self.style.font.size * 0.72, FontFamily::Monospace);
        let time_w = self.time_width(ui);
        for (y, l) in &laid {
            let e = self.store.entry(l.id);
            let row_rect = Rect::from_min_size(
                Pos2::new(content.left(), *y),
                Vec2::new(content.width(), l.height),
            );
            let text = self.store.text(e);
            if let Some(bg) = pal.channel_bg(e.channel) {
                clip.rect_filled(row_rect, CornerRadius::ZERO, bg);
            }
            // Whole-line highlight rules and mentions tint the row.
            let mut bar: Option<Color32> = None;
            for h in self
                .style
                .highlights
                .iter()
                .filter(|h| h.whole_line && h.applies(e.channel))
            {
                if h.re.is_match(text) || h.re.is_match(self.store.speaker(e)) {
                    clip.rect_filled(
                        row_rect,
                        CornerRadius::ZERO,
                        blend(pal.background, h.color, 0.18),
                    );
                    bar = Some(h.color);
                    break;
                }
            }
            if e.has(flags::MENTION) {
                clip.rect_filled(row_rect, CornerRadius::ZERO, pal.mention);
                bar = bar.or(Some(pal.accent));
            }
            if let Some((a, b)) = sel {
                if l.id >= a && l.id <= b {
                    clip.rect_filled(row_rect, CornerRadius::ZERO, pal.selection);
                }
            }
            if hover_row == Some(l.id)
                || state.context_row == Some(l.id) && ui.ctx().any_popup_open()
            {
                clip.rect_filled(
                    row_rect,
                    CornerRadius::ZERO,
                    pal.surface.gamma_multiply(0.35),
                );
            }
            if let Some(c) = bar {
                clip.rect_filled(
                    Rect::from_min_size(row_rect.left_top(), Vec2::new(3.0, row_rect.height())),
                    CornerRadius::ZERO,
                    c,
                );
            }
            let mut x = content.left() + LEFT_PAD;
            let ty = *y + ROW_PAD;
            if let Some(pat) = &self.style.time_pattern {
                clip.text(
                    Pos2::new(x, ty),
                    Align2::LEFT_TOP,
                    time::format(e.time, e.seconds(), pat),
                    self.style.font.clone(),
                    pal.dim,
                );
                x += time_w + 8.0;
            }
            if self.style.badges {
                let c = pal.channel(e.channel);
                let bw = text_width(ui, "WWW", &badge_font) + 6.0;
                let bh = badge_font.size + 4.0;
                let br = Rect::from_min_size(
                    Pos2::new(x, ty + (self.style.font.size - bh) * 0.5 + 1.0),
                    Vec2::new(bw, bh),
                );
                clip.rect_filled(br, CornerRadius::same(3), c.gamma_multiply(0.18));
                clip.text(
                    br.center(),
                    Align2::CENTER_CENTER,
                    e.channel.badge(),
                    badge_font.clone(),
                    c,
                );
                x += bw + 4.0;
            }
            if self.style.character_tags {
                if let Some(ch) = self.store.character(e) {
                    let h = self.style.font.size * 0.95;
                    let r = Rect::from_min_size(
                        Pos2::new(x, ty + (self.style.font.size - h) * 0.5 + 1.0),
                        Vec2::new(pill_width(ui, self.style.font.size), h),
                    );
                    paint_pill(&clip, r, pal, ch, self.style.font.size);
                }
            }
            clip.galley(
                Pos2::new(content.left() + LEFT_PAD + gutter, ty),
                l.galley.clone(),
                pal.text,
            );
        }

        if n == 0 {
            painter.text(
                content.center(),
                Align2::CENTER_CENTER,
                "No matching lines",
                FontId::proportional(self.style.font.size),
                pal.dim,
            );
        }

        // Tooltip with full date, character and file when hovering the time column.
        if let (Some(id), Some(p)) = (hover_row, ui.input(|i| i.pointer.hover_pos())) {
            if p.x < content.left() + LEFT_PAD + gutter && gutter > 0.0 {
                let e = self.store.entry(id);
                let sess = self.store.session(e.session);
                let who = sess
                    .and_then(|s| s.character.clone())
                    .unwrap_or_else(|| "unknown character".into());
                let file = sess.map(|s| s.file_name.clone()).unwrap_or_default();
                let tip = format!(
                    "{}\n{} · {}\n{}",
                    full_stamp(e),
                    who,
                    e.channel.label(),
                    file
                );
                response.clone().on_hover_text_at_pointer(tip);
            }
        }

        // ---- scrollbar -----------------------------------------------------------
        self.scrollbar(ui, state, sb_rect, n);

        // ---- "new lines" pill ------------------------------------------------------
        if !state.follow && state.unseen > 0 {
            let label = format!("⏷ {} new", state.unseen);
            let font = FontId::proportional(self.style.font.size * 0.9);
            let w = text_width(ui, &label, &font) + 20.0;
            let r = Rect::from_center_size(
                Pos2::new(content.center().x, content.bottom() - 18.0),
                Vec2::new(w, font.size + 10.0),
            );
            let resp = ui.interact(r, self.id.with("newpill"), Sense::click());
            let fill = if resp.hovered() {
                pal.accent
            } else {
                pal.accent.gamma_multiply(0.85)
            };
            painter.rect_filled(r, CornerRadius::same(255), fill);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                font,
                pal.background,
            );
            if resp.clicked() {
                state.jump_to_end();
            }
        }

        // ---- context menu ------------------------------------------------------------
        response.context_menu(|ui| {
            let Some(id) = state.context_row else {
                ui.label("No line here");
                return;
            };
            let e = self.store.entry(id);
            let speaker = self.store.speaker(e).to_string();
            let in_sel = state
                .selection()
                .map(|(a, b)| id >= a && id <= b)
                .unwrap_or(false);
            if in_sel && ui.button("Copy selected lines").clicked() {
                if let Some(t) = self.selection_text(state) {
                    ui.ctx().copy_text(t);
                }
                ui.close();
            }
            if ui.button("Copy line").clicked() {
                ui.ctx().copy_text(plain_line(self.store, e));
                ui.close();
            }
            if ui.button("Copy message").clicked() {
                ui.ctx().copy_text(self.store.text(e).to_string());
                ui.close();
            }
            let has_person =
                !speaker.is_empty() && !matches!(speaker.as_str(), "System" | "You see");
            if has_person {
                ui.separator();
                if ui.button(format!("Copy name “{speaker}”")).clicked() {
                    ui.ctx().copy_text(speaker.clone());
                    ui.close();
                }
                if ui.button(format!("Only “{speaker}” here")).clicked() {
                    actions.push(LogAction::SearchSpeaker(speaker.clone()));
                    ui.close();
                }
                if ui.button(format!("Hide “{speaker}” here")).clicked() {
                    actions.push(LogAction::ExcludeSpeaker(speaker.clone()));
                    ui.close();
                }
                if ui.button(format!("Open pane for “{speaker}”")).clicked() {
                    actions.push(LogAction::OpenSpeakerPane(speaker.clone()));
                    ui.close();
                }
                if ui.button(format!("Highlight “{speaker}”")).clicked() {
                    actions.push(LogAction::Highlight(speaker.clone()));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Select all").clicked() {
                if let (Some(a), Some(b)) = (self.rows.first(), self.rows.last()) {
                    state.sel_anchor = Some(*a);
                    state.sel_cursor = Some(*b);
                }
                ui.close();
            }
            if ui.button("Jump to newest").clicked() {
                state.jump_to_end();
                ui.close();
            }
        });

        actions
    }

    fn scrollbar(&self, ui: &mut egui::Ui, state: &mut LogState, track: Rect, n: usize) {
        let pal = self.style.palette;
        let resp = ui.interact(track, self.id.with("scrollbar"), Sense::click_and_drag());
        let visible = state.visible_rows.max(1);
        if n <= visible && state.top == 0 {
            return; // everything fits
        }
        let max_top = n.saturating_sub(visible).max(1);
        let frac = if state.follow {
            1.0
        } else {
            (state.top as f32 / max_top as f32).clamp(0.0, 1.0)
        };
        // `clamp` panics when min > max, which happens for very short panes.
        let thumb_h = (track.height() * visible as f32 / n.max(1) as f32)
            .max(24.0_f32.min(track.height()))
            .min(track.height())
            .max(0.0);
        let travel = (track.height() - thumb_h).max(0.0);
        let thumb = Rect::from_min_size(
            Pos2::new(track.left() + 2.0, track.top() + travel * frac),
            Vec2::new(track.width() - 4.0, thumb_h),
        );
        let active = resp.hovered() || resp.dragged();
        ui.painter()
            .rect_filled(track, CornerRadius::ZERO, pal.panel.gamma_multiply(0.6));
        ui.painter().rect_filled(
            thumb,
            CornerRadius::same(4),
            if active {
                pal.accent.gamma_multiply(0.8)
            } else {
                pal.dim.gamma_multiply(0.6)
            },
        );

        if let Some(p) = resp.interact_pointer_pos() {
            if resp.drag_started() || resp.clicked() {
                state.thumb_grab = Some(if thumb.contains(p) {
                    p.y - thumb.top()
                } else {
                    thumb_h * 0.5
                });
            }
            if let Some(grab) = state.thumb_grab {
                let f = ((p.y - grab - track.top()) / travel.max(1.0)).clamp(0.0, 1.0);
                if f >= 0.999 {
                    state.jump_to_end();
                } else {
                    state.follow = false;
                    state.top = ((f * max_top as f32).round() as usize).min(n.saturating_sub(1));
                    state.offset = 0.0;
                }
            }
        }
        if resp.drag_stopped() || (!resp.dragged() && !resp.is_pointer_button_down_on()) {
            state.thumb_grab = None;
        }
    }

    fn selection_text(&self, state: &LogState) -> Option<String> {
        let (a, b) = state.selection()?;
        let start = self.rows.partition_point(|&r| r < a);
        let end = self.rows.partition_point(|&r| r <= b);
        if start >= end {
            return None;
        }
        let mut out = String::new();
        for &id in &self.rows[start..end] {
            out.push_str(&plain_line(self.store, self.store.entry(id)));
            out.push('\n');
        }
        Some(out)
    }
}

/// Two-letter tag for a character: `Aldric Thorne` → AT, `SableMorrow` → SM.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut out: String = if words.len() >= 2 {
        words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect()
    } else {
        let caps: String = name.chars().filter(|c| c.is_uppercase()).take(2).collect();
        if caps.chars().count() >= 2 {
            caps
        } else {
            name.chars().take(2).collect()
        }
    };
    if let Some(first) = out.chars().next() {
        let rest: String = out.chars().skip(1).collect();
        out = first.to_uppercase().chain(rest.chars()).collect();
    }
    out
}

fn pill_width(ui: &egui::Ui, font_size: f32) -> f32 {
    text_width(
        ui,
        "WW",
        &FontId::new(font_size * 0.7, FontFamily::Proportional),
    ) + 8.0
}

fn paint_pill(painter: &egui::Painter, r: Rect, pal: &Palette, character: &str, font_size: f32) {
    let (label, c) = pal.chip(character);
    painter.rect_filled(r, CornerRadius::same(255), c.gamma_multiply(0.22));
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(font_size * 0.7, FontFamily::Proportional),
        c,
    );
}

/// Character pill as a standalone widget (status bar legend).
pub fn character_pill(ui: &mut egui::Ui, pal: &Palette, character: &str) -> egui::Response {
    let size = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Small)
        .map(|f| f.size)
        .unwrap_or(11.0)
        * 1.3;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(pill_width(ui, size), size), Sense::click());
    paint_pill(ui.painter(), r, pal, character, size);
    resp.on_hover_text(format!("{character} — click to edit chips"))
}

pub fn text_width(ui: &egui::Ui, s: &str, font: &FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
        .size()
        .x
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn character_initials() {
        // Invented names.
        assert_eq!(initials("Aldric Thorne"), "AT");
        assert_eq!(initials("SableMorrow"), "SM");
        assert_eq!(initials("quillon"), "Qu");
        assert_eq!(initials("HesperNightjar"), "HN");
        assert_eq!(initials("P"), "P");
    }
}
