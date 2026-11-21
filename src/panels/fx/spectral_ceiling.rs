//! Spectral Ceiling (`honeydew_audio::spectral_ceiling`), UIHeight 11 U: Honeydew's own panel. A tab strip (the
//! line, the times, the range) over the display #2 ([`graph`]) and the page's controls.
//!
//! The display reads like the Analyzer's Spectrum at the module's own Tilt: x is 20 Hz to 20 kHz on a log axis, y is
//! [`BOTTOM_DB`] to [`TOP_DB`]. The input's tilted level per band is the faint line, the threshold the straight line
//! across the Low…High range (the Clip's ceiling a fainter one over it), what each band is turned down by hangs from
//! the top, and the output (the level less that) is the channel-coloured line.

use eframe::egui::{Color32, Id, Rect, Shape, Stroke, Ui, pos2};
use honeydew_audio::analysis;
use honeydew_audio::spectral_ceiling::{
    ATTACK, BANDS, BANDS_PER_OCTAVE, CLIP, DEFAULTS, DELTA, DEPTH, FLOOR_DB, HIGH, LOW, RATIO,
    RELEASE, THRESHOLD, TILT, band_hz,
};

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// The display's dB span, top and bottom.
const TOP_DB: f32 = 12.0;
const BOTTOM_DB: f32 = -84.0;
/// The grid's step, dB.
const GRID_DB: f32 = 12.0;
/// The lines' widths, U.
const LINE_U: f32 = 0.07;
const FAINT_U: f32 = 0.05;

/// Param `p` of `params`, normalized (the default when it is missing or not a number).
fn value(params: &[f32], p: usize) -> f32 {
    match params.get(p) {
        Some(v) if v.is_finite() => v.clamp(0.0, 1.0),
        _ => DEFAULTS[p],
    }
}

/// Low and High, Hz, low first.
fn range(params: &[f32]) -> (f32, f32) {
    let hz = |p| 20.0 * 1000f32.powf(value(params, p));
    let (a, b) = (hz(LOW), hz(HIGH));
    (a.min(b), a.max(b))
}

/// The display: grid, the range's shade, the reductions, the input and output levels, the threshold and the clip.
fn graph(
    ui: &mut Ui,
    cx: &UiCtx,
    _id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (m, colors, params) = (cx.m, &cx.theme.colors, &module.params[..]);
    let live = live_analysis(ui, module.instance_id);
    let painter = ui.painter().with_clip_rect(rect);
    let px = 1.0 / m.ppp();
    painter.rect_filled(rect, 0.0, colors.control_fill.to_color32());
    let x_of = |hz: f32| {
        let x = ((hz / 20.0).log10() / 3.0).clamp(0.0, 1.0);
        rect.left() + x * rect.width()
    };
    let y_of = |db: f32| {
        let y = ((TOP_DB - db) / (TOP_DB - BOTTOM_DB)).clamp(0.0, 1.0);
        rect.top() + y * rect.height()
    };
    // The theme's grid ([`crate::theme::graph_grid`]): every 12 dB, and 100 Hz, 1 kHz, 10 kHz.
    let (v_line, h_line) = crate::theme::graph_grid(cx.theme, m.ppp());
    let mut db = TOP_DB - GRID_DB;
    while db > BOTTOM_DB && h_line.width > 0.0 {
        let y = m.floor_px(y_of(db));
        let rows = y..=y + h_line.width;
        painter.rect_filled(
            Rect::from_x_y_ranges(rect.x_range(), rows),
            0.0,
            h_line.color,
        );
        db -= GRID_DB;
    }
    for hz in [100.0, 1000.0, 10_000.0]
        .into_iter()
        .filter(|_| v_line.width > 0.0)
    {
        let x = m.floor_px(x_of(hz));
        let columns = x..=x + v_line.width;
        painter.rect_filled(
            Rect::from_x_y_ranges(columns, rect.y_range()),
            0.0,
            v_line.color,
        );
    }
    // Outside Low…High nothing is touched: shaded.
    let (low, high) = range(params);
    let (left, right) = (x_of(low), x_of(high));
    let shade = colors.menu_fill.to_color32().gamma_multiply(0.6);
    for (a, b) in [(rect.left(), left), (right, rect.right())] {
        if b > a {
            painter.rect_filled(Rect::from_x_y_ranges(a..=b, rect.y_range()), 0.0, shade);
        }
    }
    // While it runs, the engine's levels and reductions.
    let running = params.first().is_some_and(|&on| on >= 0.5);
    if let Some((levels, cuts)) = live.as_deref().and_then(analysis::spectral_ceiling) {
        let cut = |k: usize| match cuts[k] {
            c if running && c.is_finite() => c.max(0.0),
            _ => 0.0,
        };
        let edge = 2f32.powf(0.5 / BANDS_PER_OCTAVE);
        let per_db = rect.height() / (TOP_DB - BOTTOM_DB);
        let fill = ink.gamma_multiply(0.35);
        for k in 0..BANDS {
            let c = cut(k);
            if c > 0.05 {
                let hz = band_hz(k);
                let (a, b) = (x_of(hz / edge), x_of(hz * edge));
                let bottom = rect.top() + (c * per_db).min(rect.height());
                painter.rect_filled(Rect::from_x_y_ranges(a..=b, rect.top()..=bottom), 0.0, fill);
            }
        }
        let level = |k: usize| match levels[k] {
            l if l.is_finite() => l,
            _ => FLOOR_DB,
        };
        let line = |shift: &dyn Fn(usize) -> f32| -> Vec<_> {
            (0..BANDS)
                .map(|k| pos2(x_of(band_hz(k)), y_of(level(k) - shift(k))))
                .collect()
        };
        let faint = Stroke::new(m.u(FAINT_U), ink.gamma_multiply(0.45));
        painter.add(Shape::line(line(&|_| 0.0), faint));
        painter.add(Shape::line(line(&cut), Stroke::new(m.u(LINE_U), ink)));
    }
    // The threshold across the range, and the clip's ceiling over it.
    let threshold = 60.0f32.mul_add(value(params, THRESHOLD), -60.0);
    let text = colors.text.to_color32();
    let font = m.font_id(FontSize::Pt6);
    let y = y_of(threshold);
    painter.line_segment(
        [pos2(left, y), pos2(right, y)],
        Stroke::new(m.u(FAINT_U), text),
    );
    let label = format!("{threshold:.1} dB");
    let at = pos2(left + m.u(0.15), y - m.u(0.05));
    painter.text(at, Align2::LEFT_BOTTOM, label, font.clone(), text);
    let clip = value(params, CLIP);
    if clip < 1.0 {
        let y = y_of(threshold + 24.0 * clip);
        let stroke = Stroke::new(px, text.gamma_multiply(0.6));
        painter.line_segment([pos2(left, y), pos2(right, y)], stroke);
        let at = pos2(right - m.u(0.15), y - m.u(0.05));
        painter.text(
            at,
            Align2::RIGHT_BOTTOM,
            "Clip",
            font,
            text.gamma_multiply(0.6),
        );
    }
    Vec::new()
}

/// Pages `CEILING` (0), `TIME` (1) and `RANGE` (2).
pub(in crate::panels) fn panel() -> Panel {
    Panel::new(vstack([
        vstack([
            tabs(1, "CEILING · TIME · RANGE").h(Size::U(1.25)),
            display(2).draw(graph),
        ]),
        hstack([
            knob(3, "Threshold").p(THRESHOLD),
            knob(4, "Tilt").p(TILT),
            knob(5, "Ratio").p(RATIO),
            knob(6, "Clip").p(CLIP),
        ])
        .h(Size::U(4.0))
        .show(Show::Tab(0)),
        hstack([
            knob(7, "Attack").p(ATTACK),
            knob(8, "Release").p(RELEASE),
            knob(9, "Depth").p(DEPTH),
        ])
        .h(Size::U(4.0))
        .show(Show::Tab(1)),
        hstack([
            knob(10, "Low").p(LOW),
            knob(11, "High").p(HIGH),
            vstack([toggle_right(12, "Delta").p(DELTA).h(Size::U(1.25))]).w(Size::U(3.0)),
        ])
        .h(Size::U(4.0))
        .show(Show::Tab(2)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_reads_low_first_and_the_defaults_span_the_audio_band() {
        assert_eq!(range(&DEFAULTS), (20.0, 20_000.0));
        let mut params = DEFAULTS;
        (params[LOW], params[HIGH]) = (1.0, 0.0);
        assert_eq!(range(&params), (20.0, 20_000.0));
        assert_eq!(value(&[f32::NAN; 11], THRESHOLD), DEFAULTS[THRESHOLD]);
    }
}
