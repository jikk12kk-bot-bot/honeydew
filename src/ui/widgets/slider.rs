//! Vertical and horizontal sliders.
//! The horizontal one is the vertical one turned on its side: same strips, the track runs left to right.

use eframe::egui::{Align, Align2, Painter, Pos2, Rect, Response, Ui, pos2, vec2};

use super::{Control, Law, drag_control, growth, hovered, primary_down_on};
use crate::ui::UiCtx;

/// Alpha of the channel halo around the track while the slider is touched or hovered.
pub(super) const HALO: f32 = 0.4;
/// Width of that halo, in device pixels.
pub(super) const HALO_PX: f32 = 2.0;

/// The track: 3 t thick, centred across the body, inset 0.5 U from both ends of the body along it.
pub(super) fn track(body: Rect, horizontal: bool, t: f32, u: f32) -> Rect {
    let c = body.center();
    if horizontal {
        Rect::from_x_y_ranges(
            body.left() + 0.5 * u..=body.right() - 0.5 * u,
            c.y - 1.5 * t..=c.y + 1.5 * t,
        )
    } else {
        Rect::from_x_y_ranges(
            c.x - 1.5 * t..=c.x + 1.5 * t,
            body.top() + 0.5 * u..=body.bottom() - 0.5 * u,
        )
    }
}

/// The point at fraction `f` of the track's centre line, from its low end (bottom, or left when horizontal).
pub(super) fn point(track: Rect, horizontal: bool, f: f32) -> Pos2 {
    let c = track.center();
    if horizontal {
        Pos2::new(track.left() + f * track.width(), c.y)
    } else {
        Pos2::new(c.x, track.bottom() - f * track.height())
    }
}

/// The channel-coloured fill over the track: from the low end to the value, or from the middle when bipolar.
pub(super) fn fill(track: Rect, horizontal: bool, v: f32, bipolar: bool) -> Rect {
    let from = if bipolar { 0.5 } else { 0.0 };
    let half = if horizontal {
        vec2(0.0, track.height() / 2.0)
    } else {
        vec2(track.width() / 2.0, 0.0)
    };
    Rect::from_two_pos(point(track, horizontal, from), point(track, horizontal, v)).expand2(half)
}

/// Outer thumb radius: 4.5 t at rest, growing to 5.985 t at the far end. The inner disc is 2 t smaller.
fn thumb_radius(growth: f32, t: f32) -> f32 {
    (4.5 + 1.485 * growth) * t
}

/// The radius `r` held inside `body`: a thumb never reaches past the strip it sits in, however thin the strip is.
fn fitted_radius(r: f32, body: Rect) -> f32 {
    r.min(0.5 * body.width().min(body.height())).max(0.0)
}

/// The thumb's centre and outer radius at value `v`: it sits on the value and grows away from rest (the low end,
/// or the middle when bipolar).
fn thumb(track: Rect, horizontal: bool, v: f32, bipolar: bool, t: f32) -> (Pos2, f32) {
    (
        point(track, horizontal, v),
        thumb_radius(growth(v, bipolar), t),
    )
}

/// A vertical slider: label strip on top, optional value strip at the bottom, the track between.
pub fn v_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    slider(ui, cx, c, value, display, false)
}

/// A horizontal slider: the strips of [`v_slider`], with the track running left to right.
pub fn h_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    slider(ui, cx, c, value, display, true)
}

fn slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
    horizontal: bool,
) -> Response {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (label, mut body, _) = c.strips(m);
    let label_bottom = label.map_or(c.rect.top(), |r| r.bottom());
    let t = m.t();
    let mut track = track(body, horizontal, t, m.u(1.0));
    // Honeydew change: a value over the thumb that does not fit there (a short drum strip or send row) is not left
    // out: the label (if any) goes tight to the top, the value under it, and the track moves down below both.
    let font = m.font_id(c.label_font);
    let text_h = |s: &str| {
        let galley = ui
            .painter()
            .layout_no_wrap(s.to_owned(), font.clone(), c.channel);
        galley.size().y
    };
    let value_h = text_h("0");
    let compact = horizontal
        && c.value_above.is_some()
        && track.center().y - thumb_radius(1.0, t) - m.u(ABOVE_GAP) - value_h < label_bottom;
    let label_h = if compact && !c.label.is_empty() {
        text_h(c.label)
    } else {
        0.0
    };
    if compact {
        let rest = thumb_radius(0.0, t);
        let want = c.rect.top() + label_h + value_h + m.u(ABOVE_GAP) + rest;
        let y = want.min(c.rect.bottom() - rest).max(track.center().y);
        track = track.translate(vec2(0.0, y - track.center().y));
        body = c.rect.with_min_y(c.rect.top() + label_h);
    }
    let len = if horizontal {
        track.width()
    } else {
        track.height()
    };
    let law = Law::Track { len, horizontal };
    let (response, v) = drag_control(ui, cx, c, value, law, &display);
    let painter = ui.painter();
    if compact {
        if label_h > 0.0 {
            let at = pos2(c.rect.center().x, c.rect.top());
            painter.text(at, Align2::CENTER_TOP, c.label, font.clone(), c.channel);
        }
    } else {
        c.strip_texts(painter, m, v, &display);
    }

    let (body_ink, ink) = match c.flat {
        Some(flat) => (flat, flat),
        None if c.disabled => (
            colors.control_fill.to_color32(),
            colors.menu_fill.to_color32(),
        ),
        None => (colors.control_fill.to_color32(), c.channel),
    };
    if primary_down_on(ui, &response) || hovered(ui, &response) {
        // The halo stays in the strip: 2 device pixels out from the track, or the strip's own edge (and never out of
        // the clip the control was given).
        let inside = painter.clip_rect().intersect(body);
        painter.with_clip_rect(inside).rect_filled(
            track.expand(HALO_PX / m.ppp()),
            0.0,
            c.channel.gamma_multiply(HALO),
        );
    }
    if horizontal && let Some(align) = c.value_above {
        if compact {
            let top = c.rect.top() + label_h;
            value_at(painter, cx, c, track, top + value_h, align, display(v));
        } else {
            value_above(painter, cx, c, track, label_bottom, align, display(v));
        }
    }
    painter.rect_filled(track, 0.0, body_ink);
    painter.rect_filled(fill(track, horizontal, v, c.bipolar), 0.0, ink);
    let (center, r) = thumb(track, horizontal, v, c.bipolar, t);
    let r = if compact {
        r.min(c.rect.bottom() - center.y).max(0.0)
    } else {
        fitted_radius(r, body)
    };
    painter.circle_filled(center, r, body_ink);
    if r > 2.0 * t {
        painter.circle_filled(center, r - 2.0 * t, ink);
    }
    response
}

/// The gap between a value text above the thumb and the thumb's largest reach, in U.
const ABOVE_GAP: f32 = 0.1;

/// Honeydew change: a horizontal slider's value `text` just above its thumb ([`Control::value_above`]): its bottom
/// [`ABOVE_GAP`] over the largest thumb's top, along `track`'s left end, centre or right end (`align`), in the label's
/// font and the channel colour; where it would reach above `top` (the label strip's bottom, or the rect's top), the
/// slider takes its compact layout instead ([`Control::value_above`]).
fn value_above(
    painter: &Painter,
    cx: &UiCtx,
    c: &Control,
    track: Rect,
    top: f32,
    align: Align,
    text: String,
) {
    let m = cx.m;
    if text.is_empty() {
        return;
    }
    let bottom = track.center().y - thumb_radius(1.0, m.t()) - m.u(ABOVE_GAP);
    let galley = painter.layout_no_wrap(text.clone(), m.font_id(c.label_font), c.channel);
    if bottom - galley.size().y < top {
        return;
    }
    value_at(painter, cx, c, track, bottom, align, text);
}

/// A slider's value `text` with its bottom at `bottom`, along `track`'s left end, centre or right end (`align`), in
/// the label's font and the channel colour, on whole pixels.
fn value_at(
    painter: &Painter,
    cx: &UiCtx,
    c: &Control,
    track: Rect,
    bottom: f32,
    align: Align,
    text: String,
) {
    let m = cx.m;
    let galley = painter.layout_no_wrap(text, m.font_id(c.label_font), c.channel);
    let (x, anchor) = match align {
        Align::Min => (track.left(), Align2::LEFT_BOTTOM),
        Align::Center => (track.center().x, Align2::CENTER_BOTTOM),
        Align::Max => (track.right(), Align2::RIGHT_BOTTOM),
    };
    let rect = anchor.anchor_size(pos2(x, bottom), galley.size());
    painter.galley(
        pos2(m.round_px(rect.left()), m.round_px(rect.top())),
        galley,
        c.channel,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    #[test]
    fn instrument_spec_check_at_u_85() {
        // U = 85 px (t = 6): track 18 × 126 px, thumb 27 / 15 px at rest.
        let body = rect(0.0, 0.0, 100.0, 126.0 + 85.0);
        let tr = track(body, false, 6.0, 85.0);
        assert_eq!(tr, rect(41.0, 42.5, 59.0, 168.5));
        assert_eq!(thumb_radius(0.0, 6.0), 27.0);
        assert_eq!(thumb_radius(0.0, 6.0) - 2.0 * 6.0, 15.0);
        // Turned on its side.
        let body = rect(0.0, 0.0, 126.0 + 85.0, 100.0);
        assert_eq!(track(body, true, 6.0, 85.0), rect(42.5, 41.0, 168.5, 59.0));
    }

    #[test]
    fn thumb_grows_to_5_985_t_at_the_far_end() {
        assert!(near(thumb_radius(1.0, 6.0), 5.985 * 6.0));
        assert!(near(thumb_radius(0.5, 2.0), (4.5 + 1.485 / 2.0) * 2.0));
    }

    #[test]
    fn bipolar_thumbs_grow_symmetrically_from_the_middle() {
        let t = 2.0;
        for (tr, horizontal) in [
            (rect(0.0, 0.0, 4.0, 100.0), false),
            (rect(0.0, 0.0, 100.0, 4.0), true),
        ] {
            let at = |v| thumb(tr, horizontal, v, true, t);
            // At rest in the middle: 4.5 t.
            assert_eq!(at(0.5), (point(tr, horizontal, 0.5), 9.0));
            // A quarter of the range either side: half the growth, the same both ways.
            let (lo, hi) = (at(0.25), at(0.75));
            let half = (4.5 + 1.485 / 2.0) * t;
            assert!(near(lo.1, half) && near(hi.1, half), "{lo:?} {hi:?}");
            assert_eq!(lo.0, point(tr, horizontal, 0.25));
            assert_eq!(hi.0, point(tr, horizontal, 0.75));
            // Full size at both ends.
            assert!(near(at(0.0).1, 5.985 * t) && near(at(1.0).1, 5.985 * t));
            // Unipolar for contrast: rest at the low end, a quarter of the growth at 0.25.
            let (_, r) = thumb(tr, horizontal, 0.25, false, t);
            assert!(near(r, (4.5 + 1.485 / 4.0) * t), "{r}");
            assert_eq!(thumb(tr, horizontal, 0.0, false, t).1, 9.0);
        }
    }

    #[test]
    fn the_thumb_sits_at_the_value_from_the_low_end() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(point(tr, false, 0.0), pos2(2.0, 100.0));
        assert_eq!(point(tr, false, 0.25), pos2(2.0, 75.0));
        assert_eq!(point(tr, false, 1.0), pos2(2.0, 0.0));
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(point(tr, true, 0.0), pos2(0.0, 2.0));
        assert_eq!(point(tr, true, 0.25), pos2(25.0, 2.0));
        assert_eq!(point(tr, true, 1.0), pos2(100.0, 2.0));
    }

    #[test]
    fn a_thumb_never_reaches_past_its_strip() {
        // A strip 12 px across holds a thumb of radius 6 at most, however far it grew: the row keeps its own mark.
        let body = rect(0.0, 0.0, 100.0, 12.0);
        assert_eq!(fitted_radius(9.0, body), 6.0);
        assert_eq!(fitted_radius(4.0, body), 4.0);
        // Turned on its side the narrow way is the width.
        assert_eq!(fitted_radius(9.0, rect(0.0, 0.0, 8.0, 300.0)), 4.0);
        // A strip too thin for the inner disc still leaves a radius to draw with, never a negative one.
        assert_eq!(fitted_radius(9.0, rect(0.0, 0.0, 100.0, 0.0)), 0.0);
    }

    #[test]
    fn unipolar_fill_runs_from_the_low_end_to_the_value() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(fill(tr, false, 0.25, false), rect(0.0, 75.0, 4.0, 100.0));
        assert_eq!(fill(tr, false, 1.0, false), tr);
        assert_eq!(fill(tr, false, 0.0, false).height(), 0.0);
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(fill(tr, true, 0.25, false), rect(0.0, 0.0, 25.0, 4.0));
    }

    #[test]
    fn bipolar_fill_runs_from_the_middle_to_the_value() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(fill(tr, false, 0.25, true), rect(0.0, 50.0, 4.0, 75.0));
        assert_eq!(fill(tr, false, 0.75, true), rect(0.0, 25.0, 4.0, 50.0));
        assert_eq!(fill(tr, false, 0.5, true).height(), 0.0);
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(fill(tr, true, 0.25, true), rect(25.0, 0.0, 50.0, 4.0));
        assert_eq!(fill(tr, true, 1.0, true), rect(50.0, 0.0, 100.0, 4.0));
    }
}
