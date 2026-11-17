//! The Displayer's panel and window (our own addition): the transport's Displayer button opens the panel, a floating
//! box with no backdrop over the views' right side: the source (None, Image, Video), the file and `Choose…` (`Relink`
//! when it is gone), a video's trim (which scrubs the picture while dragged), `Loop`, `Mute` and its volume, where it
//! shows (Window, Background, Hidden), its opacity, the fit and Mini / Maxi. Only the Displayer button opens it; a drag
//! on it (off its controls) moves it.
//!
//! The media shows whether the panel is open or not ([`over_views`], which the app calls after the views): its window,
//! Mini, which a drag moves and its corner resizes, or Maxi, which lets every tap through to the views; or the
//! picture behind the views ([`under_views`] keeps its place under them), which takes no input: the views' backdrops
//! turn see-through over it, their notes, clips and controls drawn on top.

use eframe::egui::layers::ShapeIdx;
use eframe::egui::{self, Align2, Color32, Id, LayerId, Rect, Sense, Shape, pos2, vec2};

use super::{AppCtx, Nav};
use crate::displayer::media::{Media, Status};
use crate::displayer::{Display, DisplayerSettings, Fit, Source, placed, song_secs};
use crate::metrics::FontSize;
use crate::ui::UiCtx;
use crate::ui::widgets::{Button, Control, ParamMenu, button, h_slider};

/// The panel's width, in U.
const PANEL_W: f32 = 11.0;
/// A row's height and the gap under it, in U.
const ROW: f32 = 1.0;
const GAP: f32 = 0.25;
/// The panel's inset, in U.
const PAD: f32 = 0.4;
/// The window's resize corner, in U.
const CORNER: f32 = 1.0;
/// The window's place before any: the work area's lower right.
const WINDOW: [f32; 4] = [0.58, 0.5, 0.4, 0.45];
/// Where the panel's rect is kept for [`super::covered`].
const PANEL_RECT: &str = "displayer panel";
/// What is left of a view's backdrop over the background picture: enough to keep a grid's rows apart.
const BACKDROP_ALPHA: f32 = 0.3;
/// A backdrop is a fill of the theme's background, control or module colour at least this share of the area wide, or
/// [`BACKDROP_TALL`] of it high: a lane, a grid, a column; a button is neither.
const BACKDROP_WIDE: f32 = 0.3;
const BACKDROP_TALL: f32 = 0.5;

/// The Displayer's run-time state: its media, the panel's scrub and a pick's error.
#[derive(Default)]
pub struct State {
    pub media: Media,
    /// The video second a trim slider's drag shows, until it is released.
    scrub: Option<f64>,
    /// The last pick's error, until the next pick.
    error: Option<String>,
    /// The background's place under the views this frame: the layer and the shape [`under_views`] kept.
    slot: Option<(LayerId, ShapeIdx)>,
}

/// Before the views: keeps the background picture's place under what they draw, filled by [`over_views`].
pub fn under_views(state: &mut State, ui: &egui::Ui, s: &DisplayerSettings) {
    state.slot = (s.display == Display::Background && s.file().is_some())
        .then(|| (ui.layer_id(), ui.painter().add(Shape::Noop)));
}

/// Each frame before the views: loads what the settings show and moves the video to the transport (`tick` at `bpm`).
pub fn frame(
    state: &mut State,
    ctx: &egui::Context,
    s: &DisplayerSettings,
    tick: u64,
    bpm: f64,
    playing: bool,
) {
    let secs = song_secs(tick, bpm);
    state.media.sync(ctx, s, secs, playing, state.scrub);
}

/// After the views: a finished pick into the settings, then the window or the background. Neither opens the panel:
/// only the transport's Displayer button does.
pub fn over_views(
    state: &mut State,
    ui: &mut egui::Ui,
    cx: &UiCtx,
    area: Rect,
    s: &mut DisplayerSettings,
) {
    if let Some((source, picked)) = state.media.take_pick() {
        match picked {
            Some(Ok(location)) => {
                state.error = None;
                let same = s.source == source && s.file() == Some(location.as_str());
                match source {
                    Source::Image => s.image = location,
                    Source::Video => {
                        if s.video != location {
                            (s.start, s.end) = (0.0, 0.0);
                        }
                        s.video = location;
                    }
                    Source::None => {}
                }
                s.source = source;
                if same {
                    state.media.reload();
                }
            }
            Some(Err(err)) => state.error = Some(err),
            None => {}
        }
    }
    let slot = state.slot.take();
    if s.file().is_none() {
        return;
    }
    let m = cx.m;
    match s.display {
        Display::Hidden => {}
        Display::Background => {
            if let (Some((layer, at)), Some(picture)) = (slot, state.media.picture()) {
                let opacity = s.background_opacity.clamp(0.05, 1.0);
                behind(ui.ctx(), cx, (layer, at), area, picture, (s.fit, opacity));
            }
        }
        Display::Window => {
            let rect = if s.maxi {
                area
            } else {
                let [l, t, w, h] = s.window.unwrap_or(WINDOW);
                let min = vec2(m.u(3.0), m.u(2.0)).min(area.size());
                let size = vec2(w * area.width(), h * area.height())
                    .max(min)
                    .min(area.size());
                let at = area.min + vec2(l * area.width(), t * area.height());
                let at = at.clamp(area.min, (area.max - size).max(area.min));
                Rect::from_min_size(at, size)
            };
            let id = Id::new("displayer window");
            // Mini takes the drags that move it; Maxi takes nothing, the views under it keep their taps.
            let body = (!s.maxi).then(|| ui.interact(rect, id, Sense::drag()));
            let corner = Rect::from_min_size(
                rect.max - vec2(m.u(CORNER), m.u(CORNER)),
                vec2(m.u(CORNER), m.u(CORNER)),
            );
            let resize = (!s.maxi).then(|| ui.interact(corner, id.with("corner"), Sense::drag()));
            let painter = ui.painter().with_clip_rect(rect.intersect(area));
            // On black, see-through by the opacity (the status, while there is no picture, on solid black).
            let k = (s.window_opacity.clamp(0.05, 1.0) * 255.0).round() as u8;
            let picture = state.media.picture();
            let under = if picture.is_some() { k } else { 255 };
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(under));
            if let Some((texture, size)) = picture {
                let to = placed((size.x, size.y), rect, s.fit);
                painter.image(texture, to, uv(), Color32::from_white_alpha(k));
            } else {
                let font = m.font_id(FontSize::Pt8);
                let text = status_text(state, s);
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    text,
                    font,
                    Color32::from_gray(160),
                );
            }
            let edge = cx.theme.colors.text.to_color32().gamma_multiply(0.5);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, edge),
                egui::StrokeKind::Inside,
            );
            if let Some(resize) = &resize {
                // The corner's grip: three short diagonals.
                for i in 1..=3 {
                    let d = m.u(CORNER) * i as f32 / 4.0;
                    let stroke = egui::Stroke::new(1.5, edge);
                    painter
                        .line_segment([rect.max - vec2(d, 2.0), rect.max - vec2(2.0, d)], stroke);
                }
                if resize.dragged() {
                    let w = s.window.get_or_insert(WINDOW);
                    let d = resize.drag_delta();
                    w[2] = (w[2] + d.x / area.width()).clamp(0.1, 1.0);
                    w[3] = (w[3] + d.y / area.height()).clamp(0.1, 1.0);
                }
            }
            let resizing = resize.as_ref().is_some_and(|r| r.dragged());
            if let Some(body) = body.filter(|b| b.dragged() && !resizing) {
                let d = body.drag_delta();
                let w = s.window.get_or_insert(WINDOW);
                w[0] = (w[0] + d.x / area.width()).clamp(0.0, (1.0 - w[2]).max(0.0));
                w[1] = (w[1] + d.y / area.height()).clamp(0.0, (1.0 - w[3]).max(0.0));
            }
        }
    }
}

/// The background: the picture at `opacity` into the place kept under the views (`at` in `layer`), and the views'
/// backdrops drawn since made see-through over it ([`BACKDROP_ALPHA`]), so that their notes, clips and controls show
/// on the picture.
fn behind(
    ctx: &egui::Context,
    cx: &UiCtx,
    (layer, at): (LayerId, ShapeIdx),
    area: Rect,
    (texture, size): (egui::TextureId, egui::Vec2),
    (fit, opacity): (Fit, f32),
) {
    let c = &cx.theme.colors;
    let backdrops = [
        c.background.to_color32(),
        c.control_fill.to_color32(),
        c.module_fill.to_color32(),
    ];
    let (wide, tall) = (area.width() * BACKDROP_WIDE, area.height() * BACKDROP_TALL);
    let tint = Color32::from_white_alpha((opacity * 255.0).round() as u8);
    let picture = Shape::image(texture, placed((size.x, size.y), area, fit), uv(), tint);
    ctx.graphics_mut(|g| {
        let list = g.entry(layer);
        let end = list.next_idx().0;
        if at.0 >= end {
            return;
        }
        for i in at.0 + 1..end {
            list.mutate_shape(ShapeIdx(i), |clipped| {
                see_through(&mut clipped.shape, &backdrops, wide, tall);
            });
        }
        list.set(at, area, picture);
    });
}

/// A backdrop in `shape` (see [`BACKDROP_WIDE`]) made see-through.
fn see_through(shape: &mut Shape, backdrops: &[Color32], wide: f32, tall: f32) {
    match shape {
        Shape::Vec(shapes) => {
            for s in shapes {
                see_through(s, backdrops, wide, tall);
            }
        }
        Shape::Rect(r) => {
            let big = r.rect.width() >= wide || r.rect.height() >= tall;
            if big && r.brush.is_none() && backdrops.contains(&r.fill) {
                r.fill = r.fill.gamma_multiply(BACKDROP_ALPHA);
            }
        }
        _ => {}
    }
}

/// The panel's rect last drawn, for [`super::covered`].
pub fn panel_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Rect>(Id::new(PANEL_RECT)))
}

/// The whole texture.
fn uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

/// What the media's status says.
fn status_text(state: &State, s: &DisplayerSettings) -> String {
    if let Some(err) = &state.error {
        return err.clone();
    }
    let media = &state.media;
    match &media.status {
        Status::Empty if s.source == Source::None => "Nothing shown".into(),
        Status::Empty => "No file chosen".into(),
        Status::Loading => "Loading…".into(),
        Status::Ready => {
            let size = media
                .size
                .map_or_else(String::new, |(w, h)| format!("{w}×{h}"));
            if s.source == Source::Video {
                let hw = if media.hardware { " · hardware" } else { "" };
                format!("{} · {size}{hw}", clock(media.duration))
            } else {
                size
            }
        }
        Status::NotFound => "The file is gone: Relink it".into(),
        Status::Failed(err) => format!("Cannot open: {err}"),
        Status::NoVideo => "This version shows images only".into(),
    }
}

/// Seconds as `m:ss.t`.
fn clock(secs: f64) -> String {
    let secs = secs.max(0.0);
    let tenths = (secs * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// Shows the panel. Its X, Escape or the Displayer button close it.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let ctx = ui.ctx().clone();
    let (theme, m) = (app.cx.theme, app.cx.m);
    let c = &theme.colors;
    let (text, bg, fill) = (
        c.text.to_color32(),
        c.background.to_color32(),
        c.menu_fill.to_color32(),
    );
    let accent = c.accent.to_color32();
    let s = &mut app.settings.displayer;
    let video = s.source == Source::Video;
    let rows = 9.0
        + if video { 4.0 } else { 0.0 }
        + if s.display == Display::Hidden {
            0.0
        } else {
            1.0
        };
    let want = m.u(2.0 * PAD + rows * (ROW + GAP) + 0.5);
    let area = app.regions.area;
    let width = m.u(PANEL_W).min(area.width());
    let height = want.min(area.height());
    let id = Id::new("displayer panel");
    // At the views' top right, or where its drags moved it.
    let home = Rect::from_min_size(pos2(area.right() - width, area.top()), vec2(width, height));
    let screen = ctx.content_rect();
    let rect = crate::ui::dialog::moved(&ctx, id, home, screen);
    ctx.data_mut(|d| d.insert_temp(Id::new(PANEL_RECT), rect));
    let mut close = crate::ui::close_pressed(&ctx);
    let mut scrub = None;
    let mut pick = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.set_clip_rect(rect);
            // The whole panel first, under its controls: the touches that hit none stay here (none reaches the main
            // menu button or the views under it), and a drag moves the panel.
            ui.allocate_exact_size(rect.size(), Sense::hover());
            crate::ui::dialog::drag_to_move(ui, id, rect, rect, screen);
            let painter = ui.painter().clone();
            painter.rect_filled(rect, m.u(0.15), fill);
            painter.rect_stroke(
                rect,
                m.u(0.15),
                egui::Stroke::new(1.0, text.gamma_multiply(0.3)),
                egui::StrokeKind::Inside,
            );
            let inner = rect.shrink(m.u(PAD));
            // The title and the X.
            let title = Rect::from_min_size(inner.min, vec2(inner.width(), m.u(ROW)));
            painter.text(
                title.left_center(),
                Align2::LEFT_CENTER,
                "Displayer",
                m.font_id(FontSize::Pt12),
                text,
            );
            let x = Rect::from_min_size(
                title.right_top() - vec2(m.u(ROW), 0.0),
                vec2(m.u(ROW), m.u(ROW)),
            );
            close |=
                crate::ui::buttons::close_x(ui, &app.cx, id.with("close"), x, (fill, 0.375), 0.6)
                    .clicked();
            let body = Rect::from_min_max(pos2(inner.left(), title.bottom() + m.u(GAP)), inner.max);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .show(&mut child, |ui| {
                    let cx = &app.cx;
                    let next = |ui: &mut egui::Ui| {
                        let (r, _) =
                            ui.allocate_exact_size(vec2(body.width(), m.u(ROW)), Sense::hover());
                        ui.add_space(m.u(GAP));
                        r
                    };
                    let label = |ui: &egui::Ui, r: Rect, s: &str| {
                        let (l, rest) = r.split_left_right_at_x(r.left() + r.width() * 0.3);
                        ui.painter().text(
                            l.left_center(),
                            Align2::LEFT_CENTER,
                            s,
                            m.font_id(FontSize::Pt10),
                            text,
                        );
                        rest
                    };
                    let style = Style {
                        bg,
                        corner: m.u(0.15),
                    };

                    let r = next(ui);
                    let r = label(ui, r, "Source");
                    choices(
                        ui,
                        cx,
                        &style,
                        id.with("source"),
                        r,
                        &[
                            ("None", Source::None),
                            ("Image", Source::Image),
                            ("Video", Source::Video),
                        ],
                        &mut s.source,
                    );

                    let r = next(ui);
                    if s.source != Source::None {
                        let (name_r, choose_r) = r.split_left_right_at_x(r.right() - m.u(3.0));
                        let name = s
                            .file()
                            .map_or_else(|| "No file".to_owned(), |f| state.media.file_name(f));
                        crate::ui::widgets::shrink(
                            ui.painter(),
                            m,
                            name_r,
                            Align2::LEFT_CENTER,
                            &name,
                            FontSize::Pt10.points() as u8,
                            text,
                        );
                        let relink = state.media.status == Status::NotFound;
                        let label = if relink { "Relink" } else { "Choose…" };
                        let b = Button {
                            fill_off: bg,
                            corner: m.u(0.15),
                            ..Button::new(id.with("choose"), choose_r, label, false, c)
                        };
                        if button(ui, cx, &b, relink).clicked() {
                            pick = Some(s.source);
                        }
                    }
                    let r = next(ui);
                    let status = status_text(state, s);
                    crate::ui::widgets::shrink(
                        ui.painter(),
                        m,
                        r,
                        Align2::LEFT_CENTER,
                        &status,
                        FontSize::Pt8.points() as u8,
                        text.gamma_multiply(0.7),
                    );

                    if video {
                        let duration = state.media.duration;
                        let known = duration > 0.0;
                        let trim = s.trim(duration);
                        for (n, name) in [(0, "Start"), (1, "End")] {
                            let r = next(ui);
                            let secs = if n == 0 {
                                trim.start
                            } else if known {
                                trim.end
                            } else {
                                0.0
                            };
                            let (l, rest) = r.split_left_right_at_x(r.left() + r.width() * 0.3);
                            ui.painter().text(
                                l.left_center(),
                                Align2::LEFT_CENTER,
                                format!("{name} {}", clock(secs)),
                                m.font_id(FontSize::Pt8),
                                text,
                            );
                            let mut v = if known { (secs / duration) as f32 } else { 0.0 };
                            let control = Control {
                                menu: ParamMenu::Off,
                                bipolar: false,
                                disabled: !known,
                                ..Control::new(
                                    id.with(("trim", n)),
                                    rest,
                                    "",
                                    accent,
                                    if n == 0 { 0.0 } else { 1.0 },
                                )
                            };
                            let response = h_slider(ui, cx, &control, &mut v, |_| String::new());
                            // Only a drag moves the trim: the slider's own value, rounded to f32, would creep.
                            if known && (response.dragged() || response.changed()) {
                                let at = f64::from(v.clamp(0.0, 1.0)) * duration;
                                if response.dragged() {
                                    scrub = Some(at.min((duration - 0.05).max(0.0)));
                                }
                                if n == 0 && (at - trim.start).abs() > 1e-6 {
                                    s.start = at.min(trim.end - 0.1).max(0.0);
                                } else if n == 1 && (at - trim.end).abs() > 1e-6 {
                                    // The video's end is 0, so a later length still runs to it.
                                    s.end = if at >= duration - 0.05 {
                                        0.0
                                    } else {
                                        at.max(trim.start + 0.1)
                                    };
                                }
                            }
                        }
                        let r = next(ui);
                        let (a, b) = r.split_left_right_at_x(r.center().x - m.u(0.1));
                        let b = b.with_min_x(b.left() + m.u(0.2));
                        let looped = Button {
                            fill_off: bg,
                            corner: style.corner,
                            ..Button::new(id.with("loop"), a, "Loop", false, c)
                        };
                        if button(ui, cx, &looped, s.looped).clicked() {
                            s.looped = !s.looped;
                        }
                        let muted = Button {
                            fill_off: bg,
                            corner: style.corner,
                            ..Button::new(id.with("mute"), b, "Mute", false, c)
                        };
                        if button(ui, cx, &muted, s.muted).clicked() {
                            s.muted = !s.muted;
                        }
                        let r = next(ui);
                        let rest = label(ui, r, &format!("Volume {}%", (s.volume * 100.0).round()));
                        let control = Control {
                            menu: ParamMenu::Off,
                            bipolar: false,
                            disabled: s.muted,
                            ..Control::new(id.with("volume"), rest, "", accent, 0.8)
                        };
                        h_slider(ui, cx, &control, &mut s.volume, |_| String::new());
                        s.volume = s.volume.clamp(0.0, 1.0);
                    }

                    let r = next(ui);
                    let r = label(ui, r, "Display");
                    choices(
                        ui,
                        cx,
                        &style,
                        id.with("display"),
                        r,
                        &[
                            ("Window", Display::Window),
                            ("Background", Display::Background),
                            ("Hidden", Display::Hidden),
                        ],
                        &mut s.display,
                    );
                    // The window's opacity, or the background's.
                    let opacity = match s.display {
                        Display::Window => Some(&mut s.window_opacity),
                        Display::Background => Some(&mut s.background_opacity),
                        Display::Hidden => None,
                    };
                    if let Some(opacity) = opacity {
                        let r = next(ui);
                        let percent = (*opacity * 100.0).round();
                        let rest = label(ui, r, &format!("Opacity {percent}%"));
                        let control = Control {
                            menu: ParamMenu::Off,
                            bipolar: false,
                            ..Control::new(id.with("opacity"), rest, "", accent, 1.0)
                        };
                        h_slider(ui, cx, &control, opacity, |_| String::new());
                        *opacity = opacity.clamp(0.05, 1.0);
                    }
                    let r = next(ui);
                    let r = label(ui, r, "Fit");
                    choices(
                        ui,
                        cx,
                        &style,
                        id.with("fit"),
                        r,
                        &[
                            ("Fit", Fit::Fit),
                            ("Fill", Fit::Fill),
                            ("Stretch", Fit::Stretch),
                        ],
                        &mut s.fit,
                    );
                    let r = next(ui);
                    let r = label(ui, r, "Window");
                    let mut maxi = s.maxi;
                    choices(
                        ui,
                        cx,
                        &style,
                        id.with("size"),
                        r,
                        &[("Mini", false), ("Maxi", true)],
                        &mut maxi,
                    );
                    s.maxi = maxi;
                    let r = next(ui);
                    let note =
                        "Mini: drag the window, resize it at its corner. Maxi lets taps through.";
                    crate::ui::widgets::shrink(
                        ui.painter(),
                        m,
                        r,
                        Align2::LEFT_CENTER,
                        note,
                        FontSize::Pt8.points() as u8,
                        text.gamma_multiply(0.6),
                    );
                    let r = next(ui);
                    let note =
                        "A video starts with the song; its sound is never in an audio render.";
                    crate::ui::widgets::shrink(
                        ui.painter(),
                        m,
                        r,
                        Align2::LEFT_CENTER,
                        note,
                        FontSize::Pt8.points() as u8,
                        text.gamma_multiply(0.6),
                    );
                });
        });
    state.scrub = scrub;
    if let Some(source) = pick {
        state.error = None;
        state.media.pick(source, &ctx);
    }
    if close {
        state.scrub = None;
        ctx.data_mut(|d| d.remove::<Rect>(Id::new(PANEL_RECT)));
    }
    close.then_some(Nav::Close)
}

/// The choice buttons' look.
struct Style {
    bg: Color32,
    corner: f32,
}

/// A row of equal buttons, the chosen one lit; a tap chooses. Returns whether it changed.
fn choices<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    cx: &UiCtx,
    style: &Style,
    id: Id,
    rect: Rect,
    items: &[(&str, T)],
    value: &mut T,
) -> bool {
    let gap = cx.m.u(0.15);
    let n = items.len().max(1) as f32;
    let w = (rect.width() - gap * (n - 1.0)) / n;
    let mut changed = false;
    for (i, (name, item)) in items.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + i as f32 * (w + gap), rect.top()),
            vec2(w, rect.height()),
        );
        let b = Button {
            fill_off: style.bg,
            corner: style.corner,
            font: FontSize::Pt8,
            ..Button::new(id.with(i), r, name, false, &cx.theme.colors)
        };
        if button(ui, cx, &b, *value == *item).clicked() && *value != *item {
            *value = *item;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_minutes_seconds_and_tenths() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(61.26), "1:01.3");
        assert_eq!(clock(-3.0), "0:00.0");
    }
}
