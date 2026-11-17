//! The playlist's header column, the mini mixer when expanded: one header row per track row, drawn in theme roles. A
//! primary row is `moduleFill`, a sub-track row `controlFill`, each followed by the 1 px `background` gap; the current
//! channel's rows are drawn last.
//!
//! - **Icon cell** (primary and master rows): the left 1.5 U at full row height. Only the current channel's is filled
//!   with the channel colour (its fill grows with the header's press), its icon in `moduleFill`; every other icon is
//!   in the channel colour on the row. While the column is not fully expanded the cell's right edge holds two vertical
//!   level meters, and a channel whose sub-tracks are hidden shows a down triangle at its bottom.
//! - **Expanded** (c = 1 exactly, no fade): a horizontal stack over the row: header (flex) | mute 1.5 U | `Solo` 1.5 U
//!   | pan 2.75 U | level 6 U + a 0.25 U right margin. The master has no mute or `Solo`: `Unsolo All` takes their 3 U
//!   (1.25 U tall at most, centred), so its pan and level line up with the others'. A sub-track row is header (flex) |
//!   slider 8.75 U + 0.25 U. Behind the stack the row's name (8 pt, right and top, channel colour, middle-truncated
//!   keeping 5 characters).
//! - **Bindings** (the mixer strip's): level param 0 (reset 0.85312), pan 1 (reset 0.5), mute 2 (the inverted pill: dot
//!   up in the channel colour while audible), `Solo` 3 (two-state tap, then smart solo: [`mixer::smart_solo`]);
//!   `Unsolo All` is [`mixer::unsolo_all`]; the sliders take the drag law of
//!   [`widgets::h_slider`]. A sub-track's slider shows only on an automation lane of a module param (instance ≠ 0), in
//!   `menuFill`, bound to that param with its reset value.
//!
//! The armed-record dot keeps its place and takes the icon's ink (the channel colour, `moduleFill` on the current
//! cell); the sub-tracks triangle and the current sub-track's triangle show collapsed too (they are neither controls
//! nor labels); the expanded meters are drawn under the level slider, the left peak on the lower bar.

use eframe::egui::{
    Align2, Color32, Id, Painter, Rect, Shape, Stroke, Ui, UiBuilder, pos2, vec2,
};
use honeydew_audio::device::ModuleMeter;

use crate::icons::{draw_icon, icon_edge};
use crate::metrics::{FontSize, Metrics};
use crate::midi::links;
use crate::song::{AutomationTarget, ChannelKind, LaneKind, Song};
use crate::theme::channel_color;
use crate::ui::UiCtx;
use crate::ui::ctrl_menu::Touch;
use crate::ui::layout::{Node, NodeId, Size, layout};
use crate::ui::main_screen::{TrackRow, armed_dot};
use crate::ui::mixer;
use crate::ui::widgets::{self, Button, Control, ParamMenu, Pill, toggle_on};

/// The leaves of a row. The strip controls' ids are their channel param indices.
pub const LEVEL: NodeId = 0;
pub const PAN: NodeId = 1;
pub const MUTE: NodeId = 2;
pub const SOLO: NodeId = 3;
/// The header: the icon cell and the header's touch area.
pub const HEADER: NodeId = 4;
/// A sub-track row's automation slider.
pub const SLIDER: NodeId = 5;
/// The master row's `Unsolo All`.
pub const UNSOLO: NodeId = 6;

// Sizes in U.
/// The icon cell's width, which is also the collapsed column's.
pub const CELL_W: f32 = 1.5;
const MUTE_W: f32 = 1.5;
const SOLO_W: f32 = 1.5;
const PAN_W: f32 = 2.75;
const LEVEL_W: f32 = 6.0;
const SUB_SLIDER_W: f32 = 8.75;
/// `Unsolo All`'s height at most.
const UNSOLO_H: f32 = 1.25;
/// The level slider's (and the sub-track slider's) right margin.
const END_MARGIN: f32 = 0.25;
/// The name label's inset from the row's left and right edges (and t from its top and bottom).
const LABEL_INSET: f32 = 0.5;
/// The current sub-track's triangle: its height's half and its depth.
const CURRENT_TRIANGLE: f32 = 0.4;
/// A track folder's bar down the left edge of its members' headers, and of the folder's own row (U); the bar's
/// alpha on `text`.
const FOLDER_BAR_W: f32 = 0.1;
const CLOSED_FOLDER_BAR_W: f32 = 0.25;
const FOLDER_BAR_ALPHA: f32 = 0.6;
/// The disc behind the mute pill and the `Solo` text: its radius as a share of min(w, h), and its growth when pressed.
const DISC_SHARE: f32 = 0.4;
const PRESSED_GROWTH: f32 = 1.05;
/// The expanded meters: their inset from the slider's ends, their bottom's from its bottom, their height's share.
const METER_INSET: f32 = 0.5;
const METER_BOTTOM: f32 = 0.25;
const METER_SHARE: f32 = 1.0 / 8.0;

/// What a header row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A channel's first track.
    Primary,
    /// The master's first track: no mute or `Solo`.
    Master,
    /// Any later track of a channel.
    Sub,
    /// A track folder's row: its header the whole row.
    Folder,
}

impl RowKind {
    pub fn of(song: &Song, row: &TrackRow) -> Self {
        if row.folder_row.is_some() {
            return Self::Folder;
        }
        match (row.lane, song.channels[row.channel].kind) {
            (0, ChannelKind::Master) => Self::Master,
            (0, _) => Self::Primary,
            _ => Self::Sub,
        }
    }
}

/// The parts of a header row in `row` (points; its width the column's, its height the row's less the 1 px gap):
/// [`HEADER`] alone unless `expanded` (c = 1), then the expanded stack. Solved in device pixels by the stack rules and
/// returned in points.
pub fn parts(kind: RowKind, row: Rect, expanded: bool, m: &Metrics) -> Vec<(NodeId, Rect)> {
    let fixed = |id, w| Node::leaf(id).w(Size::U(w)).visible(expanded);
    let tree = match kind {
        RowKind::Folder => Node::hstack([
            Node::leaf(HEADER),
            fixed(MUTE, MUTE_W),
            fixed(SOLO, SOLO_W).mr(END_MARGIN),
        ]),
        RowKind::Sub => Node::hstack([
            Node::leaf(HEADER),
            fixed(SLIDER, SUB_SLIDER_W).mr(END_MARGIN),
        ]),
        _ => {
            let strip = |id, w| fixed(id, w).visible(expanded && kind == RowKind::Primary);
            Node::hstack([
                Node::leaf(HEADER),
                strip(MUTE, MUTE_W),
                strip(SOLO, SOLO_W),
                fixed(UNSOLO, MUTE_W + SOLO_W).visible(expanded && kind == RowKind::Master),
                fixed(PAN, PAN_W),
                fixed(LEVEL, LEVEL_W).mr(END_MARGIN),
            ])
        }
    };
    let ppp = m.ppp();
    (layout(&tree, row * ppp, m.u_px()).into_iter())
        .map(|(id, r)| (id, r / ppp))
        .collect()
}

/// The header's touch area in `row` ([`parts`]): the whole row while collapsed or animating; at c = 1 the header's own
/// part (1.5 U on a primary row and the master's, 4.5 U on a sub-track's).
pub fn header_rect(kind: RowKind, row: Rect, expanded: bool, m: &Metrics) -> Rect {
    (parts(kind, row, expanded, m).into_iter())
        .find(|p| p.0 == HEADER)
        .map_or(row, |p| p.1)
}

/// A level meter's value for peak `p`: clamp(((20·log10 p + 60)/66)^1.6667, 0, 1), and 0 at or below 0.001.
pub fn meter(p: f32) -> f32 {
    if p.is_nan() || p <= 0.001 {
        return 0.0;
    }
    ((20.0 * p.log10() + 60.0) / 66.0)
        .max(0.0)
        .powf(1.6667)
        .min(1.0)
}

/// The column's state for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    /// The column's left edge and width (1.5 U + 12c U), in points.
    pub left: f32,
    pub width: f32,
    /// c = 1: the controls and labels show.
    pub expanded: bool,
    /// The current track, (channel, lane): its channel's cell is filled and its rows drawn last.
    pub current: (usize, usize),
    /// The current cell's vertical scale, from the header's press feedback (1 at rest).
    pub press: f32,
    /// Multitrack recording is on: an armed Audio Record channel shows its dot.
    pub multitrack: bool,
    /// This frame's meters (the channel strip's are instance 0), if the audio runs.
    pub meters: Option<&'a [ModuleMeter]>,
}

/// What the column's controls did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// The params the user changed, already written to the song: (channel, target, value). The host sends each as
    /// `SetParam`, a param edit.
    pub changes: Vec<(usize, AutomationTarget, f32)>,
    /// A bound control's press or parameter-menu pick, for the app's recent control.
    pub touch: Option<(usize, Touch)>,
}

/// Draws the header rows `rows` (each with its top edge; `row_h` points each, the last pixel the gap) of `col`,
/// clipped to `clip`, and runs their controls on `song`'s params.
pub fn show(
    ui: &mut Ui,
    cx: &UiCtx,
    song: &mut Song,
    rows: &[(TrackRow, f32)],
    row_h: f32,
    col: &Column,
    clip: Rect,
) -> Output {
    let (m, c) = (cx.m, &cx.theme.colors);
    let mut ui = ui.new_child(UiBuilder::new().max_rect(clip));
    ui.set_clip_rect(clip.intersect(ui.clip_rect()));
    let mut out = Output::default();
    let soloed = mixer::any_soloed(song);
    let mut unsolo = false;
    let current = col.current.0;
    // The current channel's rows last, on top of the others.
    let order = (rows.iter().filter(|r| r.0.channel != current))
        .chain(rows.iter().filter(|r| r.0.channel == current));
    for &(row, top) in order {
        let rect = Rect::from_x_y_ranges(
            col.left..=col.left + col.width,
            top..=top + row_h - 1.0 / m.ppp(),
        );
        if let Some(folder) = row.folder_row {
            folder_row(&mut ui, cx, song, folder, rect, col.expanded, &mut out);
            continue;
        }
        let kind = RowKind::of(song, &row);
        let ink = channel_color(song.channels[row.channel].hue, cx.theme).to_color32();
        let painter = ui.painter().clone();
        let fill = match kind {
            RowKind::Sub => c.control_fill,
            _ => c.module_fill,
        };
        painter.rect_filled(rect, 0.0, fill.to_color32());
        // A track folder's bar down its members' rows.
        if let Some(place) = row.folder {
            let w = m.round_px(m.u(FOLDER_BAR_W));
            let bar = rect.with_max_x(rect.left() + w.max(1.0 / m.ppp()));
            let hue = song.file.folders.get(place.folder).map(|f| f.hue);
            let ink = hue.map_or(c.text, |h| channel_color(h, cx.theme)).to_color32();
            let ink = ink.gamma_multiply(FOLDER_BAR_ALPHA);
            painter.rect_filled(bar, 0.0, ink);
        }
        let is_current = row.channel == current;
        let peaks = meter_peaks(col.meters, row.channel);
        if kind == RowKind::Sub {
            if (row.channel, row.lane) == col.current {
                let (x, y, d) = (rect.left(), rect.center().y, m.u(CURRENT_TRIANGLE));
                let points = vec![pos2(x, y - d), pos2(x + d, y), pos2(x, y + d)];
                let menu_fill = c.menu_fill.to_color32();
                painter.add(Shape::convex_polygon(points, menu_fill, Stroke::NONE));
            }
        } else {
            let cell = rect.with_max_x(rect.left() + m.u(CELL_W));
            let icon_ink = if is_current {
                let grown = vec2(cell.width(), cell.height() * col.press);
                painter.rect_filled(Rect::from_center_size(cell.center(), grown), 0.0, ink);
                c.module_fill.to_color32()
            } else {
                ink
            };
            let scale = (cell.height() / icon_edge(1.0, m)).min(1.0);
            draw_icon(&painter, cell, row.icon, icon_ink, scale, m);
            if col.multitrack && row.armed {
                let (center, radius) = armed_dot(cell, m);
                painter.circle_filled(center, radius, icon_ink);
            }
            let ch = &song.channels[row.channel];
            if ch.lanes.len() > 1 && ch.zipped {
                painter.add(subtracks_triangle(cell, icon_ink, m));
            }
            if !col.expanded {
                for bar in collapsed_meters(cell, peaks, m) {
                    painter.rect_filled(bar, 0.0, icon_ink);
                }
            }
        }
        if !col.expanded {
            continue;
        }
        let ch = &song.channels[row.channel];
        let name = match kind {
            RowKind::Sub => ch.lanes[row.lane].label.as_str(),
            _ => display_name(&ch.name, ch.kind),
        };
        let label = rect.shrink2(vec2(m.u(LABEL_INSET), m.t()));
        let font = m.font_id(FontSize::Pt8);
        let placed = parts(kind, rect, true, m);
        widgets::truncated(&painter, label, Align2::RIGHT_TOP, name, font, ink, 5);
        if kind == RowKind::Sub {
            sub_slider(&mut ui, cx, song, row, &placed, ink, &mut out);
        } else {
            strip(&mut ui, cx, song, row, &placed, ink, peaks, &mut out);
        }
        if let Some(&(_, r)) = placed.iter().find(|p| p.0 == UNSOLO) {
            let r =
                Rect::from_center_size(r.center(), vec2(r.width(), r.height().min(m.u(UNSOLO_H))));
            let id = control_id(row.channel, UNSOLO);
            unsolo |= mixer::unsolo_button(&mut ui, cx, id, r, soloed);
        }
    }
    let solo = |t: &AutomationTarget| t.instance_id == 0 && t.param == 3;
    let solo_taps: Vec<usize> = (out.changes.iter())
        .filter(|c| solo(&c.1))
        .map(|c| c.0)
        .collect();
    let target = AutomationTarget {
        instance_id: 0,
        param: 3,
    };
    for x in solo_taps {
        let more = mixer::smart_solo(song, x).into_iter();
        out.changes.extend(more.map(|(c, v)| (c, target, v)));
    }
    if unsolo {
        let cleared = mixer::unsolo_all(song).into_iter();
        out.changes.extend(cleared.map(|c| (c, target, 0.0)));
    }
    out
}

/// Draws folder `folder`'s row in `rect` and runs its controls: the `moduleFill` row with the wide folder bar in the
/// folder's colour at its left edge, then in the 1.5 U cell the folder glyph in that colour with the disclosure triangle
/// under it (pointing down while open, right while closed); at c = 1 (`expanded`) the folder's name at the top right
/// of its header part and its track count under it, then the folder's Mute pill and `Solo` button, drawn as a
/// primary row's, lit while every member has it ([`folder_switch`]); a tap switches every member's.
fn folder_row(
    ui: &mut Ui,
    cx: &UiCtx,
    song: &mut Song,
    folder: usize,
    rect: Rect,
    expanded: bool,
    out: &mut Output,
) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let Some(f) = song.file.folders.get(folder) else {
        return;
    };
    let painter = ui.painter().clone();
    let ink = channel_color(f.hue, cx.theme).to_color32();
    painter.rect_filled(rect, 0.0, c.module_fill.to_color32());
    let w = m.round_px(m.u(CLOSED_FOLDER_BAR_W)).max(1.0 / m.ppp());
    painter.rect_filled(rect.with_max_x(rect.left() + w), 0.0, ink);
    // The glyph: a tab and a body, in the cell's upper part.
    let cell = rect.with_max_x(rect.left() + m.u(CELL_W)).with_min_x(rect.left() + w);
    let t = m.t();
    let body = Rect::from_center_size(
        cell.center() - vec2(0.0, cell.height() * 0.12),
        vec2(cell.width() * 0.5, cell.height() * 0.32),
    );
    let tab = Rect::from_min_size(
        body.left_top() - vec2(0.0, 2.0 * t),
        vec2(body.width() * 0.45, 2.0 * t + 1.0),
    );
    painter.rect_filled(tab, t * 0.5, ink);
    painter.rect_filled(body, t * 0.5, ink);
    // The disclosure triangle.
    let (x, y, d) = (cell.center().x, body.bottom() + 5.0 * t, 2.0 * t);
    let points = match f.open {
        true => vec![pos2(x - d, y - d * 0.5), pos2(x + d, y - d * 0.5), pos2(x, y + d)],
        false => vec![pos2(x - d * 0.5, y - d), pos2(x + d, y), pos2(x - d * 0.5, y + d)],
    };
    painter.add(Shape::convex_polygon(points, ink, Stroke::NONE));
    if !expanded {
        return;
    }
    let placed = parts(RowKind::Folder, rect, true, m);
    let part = |id: NodeId| placed.iter().find(|p| p.0 == id).map(|p| p.1);
    let header = part(HEADER).unwrap_or(rect);
    let members = song.folder_members(folder).len();
    let label = (header.with_min_x(cell.right())).shrink2(vec2(m.u(LABEL_INSET), m.t()));
    let font = m.font_id(FontSize::Pt8);
    widgets::truncated(&painter, label, Align2::RIGHT_TOP, &f.name, font.clone(), ink, 5);
    let count = match members {
        1 => "1 track".to_owned(),
        n => format!("{n} tracks"),
    };
    let dim = c.text.to_color32().gamma_multiply(0.6);
    widgets::truncated(&painter, label, Align2::RIGHT_BOTTOM, &count, font, dim, 5);
    if members == 0 {
        return;
    }
    let id = |leaf: NodeId| Id::new("mini_mixer_folder").with((folder, leaf));
    if let Some(r) = part(MUTE) {
        let grow = if held(ui, r) { PRESSED_GROWTH } else { 1.0 };
        let disc = DISC_SHARE * r.width().min(r.height()) * grow;
        let fill = c.module_fill.to_color32();
        let fill = if grow > 1.0 { widgets::pressed(fill) } else { fill };
        painter.circle_filled(r.center(), disc, fill);
        let mut on = if song.folder_muted(folder) { 1.0 } else { 0.0 };
        let control = Control {
            menu: ParamMenu::Off,
            ..Control::new(id(MUTE), r, "", ink, 0.0)
        };
        let inverted = Pill {
            inverted: true,
            ..Pill::default()
        };
        if widgets::pill_toggle(ui, cx, &control, inverted, &mut on, |_| String::new()).changed() {
            out.changes.extend(folder_switch(song, folder, false));
        }
    }
    if let Some(r) = part(SOLO) {
        let grow = if held(ui, r) { PRESSED_GROWTH } else { 1.0 };
        let b = Button {
            font: FontSize::Pt8,
            shape: 2.0 * DISC_SHARE * grow,
            fill_off: c.module_fill.to_color32(),
            text_on: c.module_fill.to_color32(),
            menu: ParamMenu::Off,
            ..Button::new(id(SOLO), r, "Solo", true, c)
        };
        // The column's smart solo takes each member's solo as its own tap.
        if widgets::button(ui, cx, &b, song.folder_soloed(folder)).clicked() {
            out.changes.extend(folder_switch(song, folder, true));
        }
    }
}

/// Folder `folder`'s Mute (`solo` false: channel strip param 2) or `Solo` (param 3): on for every member unless every
/// one has it already, then off for all. Returns the params it changed, already written to `song`: (channel, target,
/// value). A solo's smart solo ([`mixer::smart_solo`]) is the caller's.
pub fn folder_switch(song: &mut Song, folder: usize, solo: bool) -> Vec<(usize, AutomationTarget, f32)> {
    let (param, all) = match solo {
        true => (3, song.folder_soloed(folder)),
        false => (2, song.folder_muted(folder)),
    };
    let to = if all { 0.0 } else { 1.0 };
    let target = AutomationTarget {
        instance_id: 0,
        param,
    };
    let mut changes = Vec::new();
    for c in song.folder_members(folder) {
        let v = &mut song.channels[c].params.0[param as usize];
        if *v != to {
            *v = to;
            changes.push((c, target, to));
        }
    }
    changes
}

/// The row's name at c = 1: the channel's name, or its kind's default (`MASTER`, `AUX` for a send, else `CHANNEL`).
fn display_name(name: &str, kind: ChannelKind) -> &str {
    match (name, kind) {
        ("", ChannelKind::Master) => "MASTER",
        ("", ChannelKind::EffectTrack) => "AUX",
        ("", _) => "CHANNEL",
        (name, _) => name,
    }
}

/// The channel strip's (instance 0) left and right peaks this frame: none without meters.
pub(crate) fn meter_peaks(meters: Option<&[ModuleMeter]>, channel: usize) -> [f32; 2] {
    let found = (meters.into_iter().flatten())
        .find(|mm| mm.key.channel == channel && mm.key.instance_id == 0);
    found.map_or([0.0; 2], |mm| [mm.value.peak_left, mm.value.peak_right])
}

/// The collapsed meters of `cell`: [`vertical_meters`] from top + 8t to bottom − 8t.
fn collapsed_meters(cell: Rect, peaks: [f32; 2], m: &Metrics) -> Vec<Rect> {
    let t = m.t();
    vertical_meters(cell.shrink2(vec2(0.0, 8.0 * t)), peaks, t)
}

/// Two meter bars t wide at `span`'s right edge (x = right − 3t and right − 2t, the left peak first), each filled from
/// its bottom up its height by its meter value. Empty bars are left out.
pub(crate) fn vertical_meters(span: Rect, peaks: [f32; 2], t: f32) -> Vec<Rect> {
    let (top, bottom) = (span.top(), span.bottom());
    (peaks.into_iter().enumerate())
        .filter(|&(_, p)| meter(p) > 0.0)
        .map(|(i, p)| {
            let x = span.right() - (3.0 - i as f32) * t;
            let y = bottom - (bottom - top).max(0.0) * meter(p);
            Rect::from_x_y_ranges(x..=x + t, y..=bottom)
        })
        .collect()
}

/// The expanded meters inside the level slider's `slider` rect: from x + 0.5 U, (w − 1 U)·m wide, h/8 tall each; the
/// lower bar (the left peak) ends 0.25 U above the slider's bottom, the upper one 1 px above the lower. Empty bars are
/// left out.
fn expanded_meters(slider: Rect, peaks: [f32; 2], m: &Metrics) -> Vec<Rect> {
    let h = slider.height() * METER_SHARE;
    let left = slider.left() + m.u(METER_INSET);
    let len = (slider.width() - m.u(2.0 * METER_INSET)).max(0.0);
    let mut bottom = slider.bottom() - m.u(METER_BOTTOM);
    let mut bars = Vec::new();
    for p in peaks {
        let w = len * meter(p);
        if w > 0.0 {
            bars.push(Rect::from_min_max(
                pos2(left, bottom - h),
                pos2(left + w, bottom),
            ));
        }
        bottom -= h + 1.0 / m.ppp();
    }
    bars
}

/// The down triangle in `cell` while the channel's sub-tracks are hidden: base from (cx − 2t, bottom − 4t) to
/// (cx + 2t, bottom − 4t), apex at (cx, bottom − 2t).
fn subtracks_triangle(cell: Rect, color: Color32, m: &Metrics) -> Shape {
    let (x, b, t) = (cell.center().x, cell.bottom(), m.t());
    let points = vec![
        pos2(x - 2.0 * t, b - 4.0 * t),
        pos2(x + 2.0 * t, b - 4.0 * t),
        pos2(x, b - 2.0 * t),
    ];
    Shape::convex_polygon(points, color, Stroke::NONE)
}

/// The primary button is down on a press that began in `rect`.
fn held(ui: &Ui, rect: Rect) -> bool {
    ui.input(|i| {
        let p = &i.pointer;
        p.primary_down() && p.press_origin().is_some_and(|o| rect.contains(o))
    })
}

/// The id of `channel`'s control `leaf`.
fn control_id(channel: usize, leaf: NodeId) -> Id {
    Id::new("mini_mixer").with((channel, leaf))
}

/// Mute, `Solo`, pan, level and the level meters of a primary or master row, bound to its channel's strip params; a
/// collapsed row's mute is the flat pill.
#[allow(clippy::too_many_arguments)]
fn strip(
    ui: &mut Ui,
    cx: &UiCtx,
    song: &mut Song,
    row: TrackRow,
    placed: &[(NodeId, Rect)],
    ink: Color32,
    peaks: [f32; 2],
    out: &mut Output,
) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let channel = row.channel;
    let ch = &mut song.channels[channel];
    let linked = links::links(&ch.file.cc_links);
    let params = &mut ch.params.0;
    for &(leaf, r) in placed.iter().filter(|p| p.0 <= SOLO) {
        let id = control_id(channel, leaf);
        let menu = ParamMenu::bound(links::has(&linked, 0, leaf as i32));
        let control = |default| Control {
            menu,
            ..Control::new(id, r, "", ink, default)
        };
        let none = |_| String::new();
        let grow = if held(ui, r) { PRESSED_GROWTH } else { 1.0 };
        let changed = match leaf {
            MUTE => {
                // An invisible `moduleFill` disc behind the pill, which shows only pressed (× 1.15, radius × 1.05).
                let disc = DISC_SHARE * r.width().min(r.height()) * grow;
                let fill = c.module_fill.to_color32();
                let fill = if grow > 1.0 {
                    widgets::pressed(fill)
                } else {
                    fill
                };
                ui.painter().circle_filled(r.center(), disc, fill);
                let inverted = Pill {
                    inverted: true,
                    ..Pill::default()
                };
                widgets::pill_toggle(ui, cx, &control(0.0), inverted, &mut params[2], none)
                    .changed()
            }
            SOLO => {
                let on = toggle_on(params[3]);
                let b = Button {
                    font: FontSize::Pt8,
                    shape: 2.0 * DISC_SHARE * grow,
                    fill_off: c.module_fill.to_color32(),
                    text_on: c.module_fill.to_color32(),
                    menu,
                    ..Button::new(id, r, "Solo", true, c)
                };
                let clicked = widgets::button(ui, cx, &b, on).clicked();
                if clicked {
                    params[3] = if on { 0.0 } else { 1.0 };
                }
                clicked
            }
            PAN => widgets::h_slider(ui, cx, &control(0.5), &mut params[1], none).changed(),
            _ => {
                for bar in expanded_meters(r, peaks, m) {
                    ui.painter().rect_filled(bar, 0.0, ink);
                }
                widgets::h_slider(ui, cx, &control(widgets::UNITY), &mut params[0], none).changed()
            }
        };
        let target = AutomationTarget {
            instance_id: 0,
            param: leaf as i32,
        };
        if changed {
            out.changes.push((channel, target, params[leaf as usize]));
        }
        if let Some(touch) = Touch::new(target, widgets::signal(ui, id)) {
            out.touch = Some((channel, touch));
        }
    }
}

/// A sub-track row's slider: only on an automation lane of a module param (instance ≠ 0), drawn with a `menuFill` track
/// and thumb rim, showing the param's live value and setting it, reset to the param's reset value.
fn sub_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    song: &mut Song,
    row: TrackRow,
    placed: &[(NodeId, Rect)],
    ink: Color32,
    out: &mut Output,
) {
    let ch = &mut song.channels[row.channel];
    let lane = &ch.lanes[row.lane];
    let Some(target) = lane
        .target
        .filter(|t| lane.kind == LaneKind::Automation && t.instance_id != 0)
    else {
        return;
    };
    let Some(&(_, r)) = placed.iter().find(|p| p.0 == SLIDER) else {
        return;
    };
    let Some(spec) = crate::app::auto_record::spec(ch, target) else {
        return;
    };
    let linked = links::links(&ch.file.cc_links);
    let Some(value) = ch.param_mut(target) else {
        return;
    };
    let id = Id::new("mini_mixer").with((row.channel, row.lane, SLIDER));
    let control = Control {
        menu: ParamMenu::bound(links::has(&linked, target.instance_id, target.param)),
        ..Control::new(id, r, "", ink, spec.reset)
    };
    // The slider's track and thumb rim are `controlFill`: lend it `menuFill` in that role.
    // ponytail: a theme clone per drawn sub-track slider; a slider colour option if it ever shows in a profile.
    let mut theme = cx.theme.clone();
    theme.colors.control_fill = theme.colors.menu_fill;
    let menu_cx = UiCtx {
        theme: &theme,
        m: cx.m,
    };
    if widgets::h_slider(ui, &menu_cx, &control, value, |_| String::new()).changed() {
        out.changes.push((row.channel, target, *value));
        if let Some(module) = ch.rack.get_mut(target.instance_id) {
            crate::panels::couple_param(module, target.param);
        }
    }
    if let Some(touch) = Touch::new(target, widgets::signal(ui, id)) {
        out.touch = Some((row.channel, touch));
    }
}

#[cfg(test)]
mod tests;
