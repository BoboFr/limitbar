//! Popup drawing. Layout from the "Gemini 4 Argon" mock-up (header with a Pin
//! pill, one card per provider, footer with Refresh / Launch at login / Quit);
//! visual language: white canvas, ink text, hairline borders, soft radii,
//! one accent colour used sparingly.

use crate::data::{fmt_ago, fmt_when, Provider, Row, UsageFile};
use chrono::{DateTime, Local};
use eframe::egui::{
    self, text::LayoutJob, vec2, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Pos2, Rect,
    Sense, Shape, Stroke, StrokeKind, TextFormat, Ui, Vec2,
};
use std::collections::HashMap;
use std::f32::consts::TAU;

pub const WIDTH: f32 = 420.0;

// Design tokens.
const CANVAS: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF); // colors.canvas
const SURFACE_SOFT: Color32 = Color32::from_rgb(0xF7, 0xF7, 0xF7); // colors.surface-soft
const SURFACE_STRONG: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2); // colors.surface-strong
const HAIRLINE: Color32 = Color32::from_rgb(0xDD, 0xDD, 0xDD); // colors.hairline
const HAIRLINE_SOFT: Color32 = Color32::from_rgb(0xEB, 0xEB, 0xEB); // colors.hairline-soft
const INK: Color32 = Color32::from_rgb(0x22, 0x22, 0x22); // colors.ink
const BODY: Color32 = Color32::from_rgb(0x3F, 0x3F, 0x3F); // colors.body
const MUTED: Color32 = Color32::from_rgb(0x6A, 0x6A, 0x6A); // colors.muted
const MUTED_SOFT: Color32 = Color32::from_rgb(0x92, 0x92, 0x92); // colors.muted-soft
const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0x38, 0x5C); // colors.primary (accent)
const ERROR: Color32 = Color32::from_rgb(0xC1, 0x35, 0x15); // colors.primary-error-text
const ON_PRIMARY: Color32 = Color32::WHITE; // colors.on-primary

// rounded.*
const R_XS: u8 = 4;
const R_SM: u8 = 8;
const R_MD: u8 = 14;
const R_FULL: u8 = 255;

// spacing.*
const SP_XS: f32 = 4.0;
const SP_SM: f32 = 8.0;
const SP_MD: f32 = 12.0;
const SP_BASE: f32 = 16.0;

/// Card internal padding (`spacing.base`).
const PAD: f32 = SP_BASE;
/// Window inner margin.
pub const MARGIN: i8 = 16;

/// Bar colour: ink normally, accent when getting close, error red at the limit.
fn level_color(percent: f32) -> Color32 {
    if percent >= 90.0 {
        ERROR
    } else if percent >= 70.0 {
        PRIMARY
    } else {
        INK
    }
}

#[derive(Default)]
pub struct Actions {
    pub refresh: bool,
    pub quit: bool,
    pub toggle_pin: bool,
    pub toggle_autostart: bool,
    pub swap: Option<usize>,
    /// Provider name and its new collapsed state.
    pub collapse: Option<(String, bool)>,
}

pub struct ViewState {
    pub pinned: bool,
    pub autostart: bool,
    pub detecting: bool,
    /// Explicit collapsed state per provider name (see `is_collapsed`).
    pub collapsed: HashMap<String, bool>,
}

/// Cards with more rows than this start collapsed.
const AUTO_COLLAPSE_ROWS: usize = 3;

impl ViewState {
    pub fn is_collapsed(&self, name: &str, rows: usize) -> bool {
        self.collapsed.get(name).copied().unwrap_or(rows > AUTO_COLLAPSE_ROWS)
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let load = |file: &str| std::fs::read(format!(r"C:\Windows\Fonts\{file}")).ok();
    if let Some(bytes) = load("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let mut bold = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = load("seguisb.ttf") {
        fonts.font_data.insert("segoe_sb".into(), egui::FontData::from_owned(bytes).into());
        bold.insert(0, "segoe_sb".into());
    }
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Paints a single line of text anchored at `pos`; returns its rect.
fn text(ui: &Ui, pos: Pos2, align: Align2, s: &str, font: FontId, color: Color32) -> Rect {
    ui.painter().text(pos, align, s, font, color)
}

/// Text with `**bold**` spans, as used in the card notes.
fn rich_job(s: &str, size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (i, part) in s.split("**").enumerate() {
        if part.is_empty() {
            continue;
        }
        let (font_id, color) = if i % 2 == 1 { (bold(size), INK) } else { (font(size), color) };
        job.append(part, 0.0, TextFormat { font_id, color, ..Default::default() });
    }
    job
}

pub fn draw(ui: &mut Ui, data: &UsageFile, view: &ViewState, now: DateTime<Local>) -> Actions {
    let mut actions = Actions::default();
    ui.spacing_mut().item_spacing = vec2(0.0, SP_MD);

    header(ui, data, view, now, &mut actions);
    for (i, p) in data.providers.iter().enumerate() {
        card(ui, i, p, view, now, &mut actions);
    }
    if data.providers.is_empty() {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
        let msg = if view.detecting {
            "Looking for AI tools on this PC…"
        } else {
            "No AI tool found (Claude Code, Codex, Antigravity)"
        };
        text(ui, r.center(), Align2::CENTER_CENTER, msg, font(14.0), MUTED);
    }
    footer(ui, view, &mut actions);
    actions
}

fn header(ui: &mut Ui, data: &UsageFile, view: &ViewState, now: DateTime<Local>, actions: &mut Actions) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    // typography.display-sm (20 / 600)
    let title = text(ui, r.left_center() + vec2(SP_XS, 0.0), Align2::LEFT_CENTER, "Subscription Limits", bold(20.0), INK);

    // Pin pill, styled like guest-favorite-badge; pinned = ink fill (selected state).
    let label = if view.pinned { "Pinned" } else { "Pin" };
    let galley = ui.painter().layout_no_wrap(label.into(), bold(11.0), INK);
    let pill = Rect::from_min_size(
        Pos2::new(title.right() + SP_SM, r.center().y - 11.0),
        vec2(galley.size().x + 20.0, 22.0),
    );
    let resp = ui.interact(pill, ui.id().with("pin"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    if view.pinned {
        ui.painter().rect_filled(pill, CornerRadius::same(R_FULL), INK);
    } else {
        let fill = if resp.hovered() { SURFACE_SOFT } else { CANVAS };
        ui.painter().rect_filled(pill, CornerRadius::same(R_FULL), fill);
        ui.painter().rect_stroke(pill, CornerRadius::same(R_FULL), Stroke::new(1.0_f32, HAIRLINE), StrokeKind::Inside);
    }
    let fg = if view.pinned { ON_PRIMARY } else { INK };
    text(ui, pill.center(), Align2::CENTER_CENTER, label, bold(11.0), fg);
    if resp.clicked() {
        actions.toggle_pin = true;
    }

    // Freshness: newest provider update. The live dot is the one accent moment.
    if view.detecting {
        text(ui, r.right_center() - vec2(SP_XS, 0.0), Align2::RIGHT_CENTER, "checking…", font(13.0), MUTED);
    } else if let Some(t) = data.providers.iter().filter_map(|p| p.updated_at).max() {
        let s = format!("updated {}", fmt_ago(t, now, true));
        let tr = text(ui, r.right_center() - vec2(SP_XS, 0.0), Align2::RIGHT_CENTER, &s, font(13.0), MUTED);
        let stale = (now - t).num_minutes() > 30;
        let dot = if stale { MUTED_SOFT } else { PRIMARY };
        ui.painter().circle_filled(Pos2::new(tr.left() - 9.0, r.center().y), 3.5, dot);
    }
}

fn card(ui: &mut Ui, index: usize, p: &Provider, view: &ViewState, now: DateTime<Local>, actions: &mut Actions) {
    let Some(acc) = p.active_account() else { return };
    let others: Vec<_> = p.accounts.iter().enumerate().filter(|(i, _)| *i != p.active.min(p.accounts.len() - 1)).collect();

    // Note pill content: extras of the active account, other accounts, free text.
    let mut note_parts: Vec<String> = acc.extras.iter().map(|e| format!("{} **{:.0}%**", e.label, e.percent)).collect();
    note_parts.extend(others.iter().map(|(_, a)| a.summary()));
    note_parts.extend(p.note.clone());
    let note = note_parts.join(" · ");
    let can_swap = !others.is_empty();

    // Collapsed: only the most used limit, no note.
    let collapsible = acc.rows.len() > 1 || !note.is_empty();
    let collapsed = collapsible && view.is_collapsed(&p.name, acc.rows.len());
    let rows: Vec<_> = if collapsed {
        acc.rows.iter().max_by(|a, b| a.percent.total_cmp(&b.percent)).into_iter().collect()
    } else {
        acc.rows.iter().collect()
    };
    let note = if collapsed { String::new() } else { note };

    let row_h = 28.0;
    let group_h = 24.0;
    let items = layout_rows(&rows);
    let rows_h: f32 = items.iter().map(|it| if matches!(it, Item::Group(_)) { group_h } else { row_h }).sum();
    let mut h = PAD + 32.0 + SP_SM + rows_h + PAD;
    if rows.is_empty() && note.is_empty() {
        h -= SP_SM;
    }
    if !note.is_empty() {
        h += SP_SM + 34.0;
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let painter = ui.painter();
    // property-card: canvas surface, hairline border, rounded.md.
    painter.rect_filled(r, CornerRadius::same(R_MD), CANVAS);
    painter.rect_stroke(r, CornerRadius::same(R_MD), Stroke::new(1.0_f32, HAIRLINE), StrokeKind::Inside);

    let accent = rgb(p.accent_rgb());
    let inner = r.shrink(PAD);

    // Header line: icon-button-circle, name, plan, freshness.
    let tile = Rect::from_min_size(inner.min, vec2(32.0, 32.0));
    painter.rect_filled(tile, CornerRadius::same(R_FULL), SURFACE_STRONG);
    draw_glyph(ui, &p.icon, &p.name, tile.center(), 9.0, accent);

    let cy = tile.center().y;
    // typography.title-md (16 / 600)
    let name = text(ui, Pos2::new(tile.right() + SP_MD, cy), Align2::LEFT_CENTER, &p.name, bold(16.0), INK);
    let mut subtitle = acc.title();
    if collapsed && acc.rows.len() > 1 {
        subtitle = format!("{subtitle} · {} limits", acc.rows.len());
    }

    // The whole header line toggles the card; a chevron shows the state.
    let mut right = inner.right();
    if collapsible {
        let header = Rect::from_min_max(Pos2::new(r.left(), r.top()), Pos2::new(r.right(), tile.bottom() + 4.0));
        let resp = ui
            .interact(header, ui.id().with(("collapse", index)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let color = if resp.hovered() { INK } else { MUTED };
        chevron(ui, Pos2::new(inner.right() - 5.0, cy), collapsed, color);
        right -= 18.0;
        if resp.clicked() {
            actions.collapse = Some((p.name.clone(), !collapsed));
        }
    }
    if let Some(t) = p.updated_at {
        let s = format!("updated {}", fmt_ago(t, now, false));
        right = text(ui, Pos2::new(right, cy), Align2::RIGHT_CENTER, &s, font(13.0), MUTED).left();
    }
    // Subtitle gets whatever room is left between the name and "updated".
    let room = right - SP_SM - (name.right() + SP_SM);
    let subtitle = elide(painter, &subtitle, font(13.0), room);
    text(ui, Pos2::new(name.right() + SP_SM, cy + 1.0), Align2::LEFT_CENTER, &subtitle, font(13.0), MUTED);

    // Rows: label | bar | percent | reset. Variants of one model (efforts)
    // sit under a model heading, indented.
    const INDENT: f32 = SP_BASE;
    let label_w = items
        .iter()
        .filter_map(|it| match it {
            Item::Row { label, indent, .. } => {
                let w = painter.layout_no_wrap(label.clone(), font(14.0), BODY).size().x;
                Some(w + SP_MD + if *indent { INDENT } else { 0.0 })
            }
            Item::Group(_) => None,
        })
        .fold(80.0, f32::max)
        .min(118.0);
    let mut y = tile.bottom() + SP_SM;
    for item in &items {
        let (row, label, indent) = match item {
            Item::Group(model) => {
                let mid = y + group_h / 2.0 + 1.0;
                // typography.caption (14 / 500)
                text(ui, Pos2::new(inner.left(), mid), Align2::LEFT_CENTER, model, bold(14.0), INK);
                y += group_h;
                continue;
            }
            Item::Row { row, label, indent } => (row, label, *indent),
        };
        let mid = y + row_h / 2.0;
        let x0 = inner.left() + if indent { INDENT } else { 0.0 };
        if indent {
            // Tree guide linking the effort to its model heading.
            let guide = Stroke::new(1.0_f32, HAIRLINE);
            painter.line_segment([Pos2::new(inner.left() + 5.0, y - 4.0), Pos2::new(inner.left() + 5.0, mid)], guide);
            painter.line_segment([Pos2::new(inner.left() + 5.0, mid), Pos2::new(x0 - SP_XS, mid)], guide);
        }
        // typography.body-sm (14 / 400) in colors.body
        let shown = elide(painter, label, font(14.0), inner.left() + label_w - x0 - SP_SM);
        text(ui, Pos2::new(x0, mid), Align2::LEFT_CENTER, &shown, font(14.0), BODY);

        let bar_w = 96.0 - (label_w - 80.0);
        let bar = Rect::from_min_size(Pos2::new(inner.left() + label_w, mid - 3.0), vec2(bar_w, 6.0));
        let pct = row.percent.clamp(0.0, 100.0);
        painter.rect_filled(bar, CornerRadius::same(R_FULL), HAIRLINE_SOFT);
        let fill_w = (bar.width() * pct / 100.0).max(if pct > 0.0 { 6.0 } else { 0.0 });
        if fill_w > 0.0 {
            let fill = Rect::from_min_size(bar.min, vec2(fill_w, bar.height()));
            painter.rect_filled(fill, CornerRadius::same(R_FULL), level_color(pct));
        } else {
            painter.circle_filled(Pos2::new(bar.left() + 3.0, mid), 2.5, MUTED_SOFT);
        }

        let pct_s = format!("{:.0}%", row.percent);
        text(ui, Pos2::new(bar.right() + 44.0, mid), Align2::RIGHT_CENTER, &pct_s, bold(14.0), INK);

        let detail = match (&row.detail, row.resets_at) {
            (Some(d), _) => d.clone(),
            (None, Some(t)) => format!("{} {}", row.verb.as_deref().unwrap_or("resets"), fmt_when(t, now)),
            (None, None) => String::new(),
        };
        // typography.caption-sm (13 / 400)
        text(ui, Pos2::new(inner.right(), mid), Align2::RIGHT_CENTER, &detail, font(13.0), MUTED);
        y += row_h;
    }

    // Note pill with optional Swap button.
    if !note.is_empty() {
        let nr = Rect::from_min_max(Pos2::new(inner.left(), y + SP_SM), Pos2::new(inner.right(), y + SP_SM + 34.0));
        painter.rect_filled(nr, CornerRadius::same(R_SM), SURFACE_SOFT);
        let mut right = nr.right() - SP_MD;
        if can_swap {
            // button-secondary, compact: canvas fill, ink outline, rounded.sm.
            let g = painter.layout_no_wrap("Swap".into(), bold(13.0), INK);
            let br = Rect::from_center_size(
                Pos2::new(nr.right() - 6.0 - (g.size().x + 20.0) / 2.0, nr.center().y),
                vec2(g.size().x + 20.0, 24.0),
            );
            let resp = ui.interact(br, ui.id().with(("swap", index)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            painter.rect_filled(br, CornerRadius::same(R_SM), if resp.hovered() { SURFACE_STRONG } else { CANVAS });
            painter.rect_stroke(br, CornerRadius::same(R_SM), Stroke::new(1.0_f32, INK), StrokeKind::Inside);
            text(ui, br.center(), Align2::CENTER_CENTER, "Swap", bold(13.0), INK);
            if resp.clicked() {
                actions.swap = Some(index);
            }
            right = br.left() - SP_SM;
        }
        let mut job = rich_job(&note, 13.0, MUTED);
        job.wrap.max_width = right - (nr.left() + SP_MD);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = painter.layout_job(job);
        let pos = Pos2::new(nr.left() + SP_MD, nr.center().y - galley.size().y / 2.0);
        painter.galley(pos, galley, MUTED);
    }
}

fn footer(ui: &mut Ui, view: &ViewState, actions: &mut Actions) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.top() - SP_XS, Stroke::new(1.0_f32, HAIRLINE));
    let w = r.width() / 3.0;
    let cell = |i: f32| Rect::from_min_size(Pos2::new(r.left() + i * w, r.top() + SP_XS), vec2(w, r.height() - SP_XS));
    // typography.button-sm (14 / 500), shortcuts in caption-sm muted.
    let label_font = || bold(14.0);

    // Refresh
    let c0 = cell(0.0);
    let resp = ui.interact(c0, ui.id().with("refresh"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    hover_bg(ui, &resp, c0);
    let ic = Pos2::new(c0.left() + SP_BASE, c0.center().y);
    refresh_glyph(ui, ic, 5.5, INK);
    let t = text(ui, ic + vec2(SP_MD, 0.0), Align2::LEFT_CENTER, "Refresh", label_font(), INK);
    text(ui, t.right_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, "Ctrl+R", font(12.0), MUTED_SOFT);
    if resp.clicked() {
        actions.refresh = true;
    }

    // Launch at login
    let c1 = cell(1.0);
    let resp = ui.interact(c1, ui.id().with("login"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    hover_bg(ui, &resp, c1);
    let label_w = ui.painter().layout_no_wrap("Launch at login".into(), label_font(), INK).size().x;
    let box_r = Rect::from_center_size(Pos2::new(c1.center().x - (label_w + 24.0) / 2.0 + 8.0, c1.center().y), Vec2::splat(16.0));
    if view.autostart {
        // Selected state: ink fill, like date-picker-day-selected.
        ui.painter().rect_filled(box_r, CornerRadius::same(R_XS), INK);
        let pts = vec![
            box_r.left_center() + vec2(3.5, 0.5),
            box_r.center() + vec2(-1.0, 4.0),
            box_r.right_top() + vec2(-3.5, 4.5),
        ];
        ui.painter().add(Shape::line(pts, Stroke::new(1.8_f32, ON_PRIMARY)));
    } else {
        ui.painter().rect_stroke(box_r, CornerRadius::same(R_XS), Stroke::new(1.0_f32, MUTED_SOFT), StrokeKind::Inside);
    }
    text(ui, box_r.right_center() + vec2(SP_SM, 0.0), Align2::LEFT_CENTER, "Launch at login", label_font(), INK);
    if resp.clicked() {
        actions.toggle_autostart = true;
    }

    // Quit
    let c2 = cell(2.0);
    let resp = ui.interact(c2, ui.id().with("quit"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    hover_bg(ui, &resp, c2);
    let k = text(ui, c2.right_center() - vec2(SP_MD, 0.0), Align2::RIGHT_CENTER, "Ctrl+Q", font(12.0), MUTED_SOFT);
    text(ui, k.left_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, "Quit", label_font(), INK);
    if resp.clicked() {
        actions.quit = true;
    }
}

enum Item<'a> {
    /// Model heading above its effort variants.
    Group(String),
    Row { row: &'a Row, label: String, indent: bool },
}

const EFFORTS: [&str; 7] = ["minimal", "low", "medium", "high", "xhigh", "max", "thinking"];

/// Splits "Gemini 3 Pro (High)" / "Gemini 3 Pro - High" / "GPT-5 high" into
/// the model and its effort.
fn split_variant(label: &str) -> Option<(String, String)> {
    let l = label.trim();
    if let Some(open) = l.rfind(" (").filter(|_| l.ends_with(')')) {
        return Some((l[..open].trim().to_string(), l[open + 2..l.len() - 1].trim().to_string()));
    }
    if let Some(i) = l.rfind(" - ") {
        return Some((l[..i].trim().to_string(), l[i + 3..].trim().to_string()));
    }
    let (base, last) = l.rsplit_once(' ')?;
    EFFORTS
        .contains(&last.to_ascii_lowercase().as_str())
        .then(|| (base.trim().to_string(), last.to_string()))
}

/// Groups rows of the same model when it has several variants; other rows
/// keep their full label. Groups appear where their first row was.
fn layout_rows<'a>(rows: &[&'a Row]) -> Vec<Item<'a>> {
    let parsed: Vec<_> = rows.iter().map(|r| split_variant(&r.label)).collect();
    let count = |model: &str| parsed.iter().flatten().filter(|(m, _)| m == model).count();
    let mut items = Vec::new();
    let mut done = vec![false; rows.len()];
    for i in 0..rows.len() {
        if done[i] {
            continue;
        }
        match &parsed[i] {
            Some((model, _)) if count(model) > 1 => {
                items.push(Item::Group(model.clone()));
                let mut variants: Vec<(usize, &String)> = (i..rows.len())
                    .filter_map(|j| match &parsed[j] {
                        Some((m, effort)) if m == model => Some((j, effort)),
                        _ => None,
                    })
                    .collect();
                variants.sort_by_key(|(j, effort)| (effort_rank(effort), *j));
                for (j, effort) in variants {
                    done[j] = true;
                    items.push(Item::Row { row: rows[j], label: capitalize(effort), indent: true });
                }
            }
            _ => {
                done[i] = true;
                items.push(Item::Row { row: rows[i], label: rows[i].label.clone(), indent: false });
            }
        }
    }
    items
}

/// Position of an effort in `EFFORTS` (unknown variants go last).
fn effort_rank(effort: &str) -> usize {
    let e = effort.to_ascii_lowercase();
    EFFORTS.iter().position(|x| *x == e).unwrap_or(EFFORTS.len())
}

/// Shortens `s` with an ellipsis so it fits in `max_w` points.
fn elide(painter: &egui::Painter, s: &str, font_id: FontId, max_w: f32) -> String {
    let width = |t: &str| painter.layout_no_wrap(t.to_string(), font_id.clone(), BODY).size().x;
    if width(s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    for n in (1..chars.len()).rev() {
        let t: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "…";
        if width(&t) <= max_w {
            return t;
        }
    }
    "…".into()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Small "v" (expanded) or ">" (collapsed) centred on `c`.
fn chevron(ui: &Ui, c: Pos2, collapsed: bool, color: Color32) {
    let pts = if collapsed {
        vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)]
    } else {
        vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)]
    };
    ui.painter().add(Shape::line(pts, Stroke::new(1.6_f32, color)));
}

fn hover_bg(ui: &Ui, resp: &egui::Response, r: Rect) {
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(2.0, 1.0)), CornerRadius::same(R_SM), SURFACE_SOFT);
    }
}

// ---------------------------------------------------------------------------
// Glyphs (simple vector approximations, no bundled logos)

fn refresh_glyph(ui: &Ui, c: Pos2, r: f32, color: Color32) {
    let start = -0.15 * TAU;
    let pts: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = start + (i as f32 / 24.0) * 0.8 * TAU;
            c + r * vec2(a.cos(), a.sin())
        })
        .collect();
    let end = *pts.last().unwrap();
    ui.painter().add(Shape::line(pts, Stroke::new(1.5_f32, color)));
    let a = start + 0.8 * TAU;
    let tangent = vec2(-a.sin(), a.cos());
    let normal = vec2(a.cos(), a.sin());
    let tip = end + tangent * 2.5;
    ui.painter().add(Shape::convex_polygon(
        vec![tip, end - tangent * 1.5 + normal * 2.8, end - tangent * 1.5 - normal * 2.8],
        color,
        Stroke::NONE,
    ));
}

fn draw_glyph(ui: &Ui, icon: &str, name: &str, c: Pos2, r: f32, accent: Color32) {
    let painter = ui.painter();
    match icon {
        "claude" => {
            for i in 0..12 {
                let a = i as f32 / 12.0 * TAU;
                let len = if i % 2 == 0 { r } else { r * 0.78 };
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 1.5, c + d * len], Stroke::new(1.9_f32, accent));
            }
        }
        "gemini" => {
            // Four-pointed star built from four convex kites.
            for i in 0..4 {
                let a = i as f32 / 4.0 * TAU - TAU / 4.0;
                let d = vec2(a.cos(), a.sin());
                let n = vec2(-d.y, d.x);
                let side = r * 0.3;
                painter.add(Shape::convex_polygon(
                    vec![c, c + (d + n) * side * 0.72, c + d * r * 1.1, c + (d - n) * side * 0.72],
                    accent,
                    Stroke::NONE,
                ));
            }
        }
        "openai" => {
            for i in 0..6 {
                let a = i as f32 / 6.0 * TAU;
                painter.circle_stroke(c + vec2(a.cos(), a.sin()) * r * 0.42, r * 0.5, Stroke::new(1.3_f32, INK));
            }
        }
        "grok" => {
            let s = Stroke::new(1.8_f32, INK);
            painter.line_segment([c + vec2(-r * 0.75, r * 0.85), c + vec2(r * 0.7, -r * 0.9)], s);
            painter.line_segment([c + vec2(-r * 0.7, -r * 0.35), c + vec2(-r * 0.05, r * 0.35)], s);
            painter.line_segment([c + vec2(r * 0.55, -r * 0.15), c + vec2(r * 0.55, r * 0.9)], s);
        }
        _ => {
            let initial: String = name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            painter.text(c, Align2::CENTER_CENTER, initial, bold(13.0), accent);
        }
    }
}

pub fn bg() -> Color32 {
    CANVAS
}
