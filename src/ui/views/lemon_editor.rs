//! Issue #68: a built-in Lemon plugin's editor full screen, from its rack card's full screen button. It shows as large
//! as the screen holds it, its aspect kept and centred, over the dimmed screen, painted at that size and driven as in
//! its card ([`lemon_card::show`]): taps, drags, a second finger, double taps, the wheel and its file requests (the
//! sample picker opens over it). The close X or `close` (Escape, Back) closes it, once the editor's own menu or type-in
//! box is gone.
//!
//! The virtual mouse is on as it opens: the whole screen is a touchpad that moves an arrow cursor over the editor
//! ([`lemon_card::set_mouse`]); the cursor works the editor's menus and type-in box too. The mouse disc under the close X
//! turns it off (touches press where they land, as in the card) and on again. With the mouse on, the magnifier disc
//! under it zooms in about the cursor ([`lemon_card::set_zoom`]), and the `+` and `−` discs on the left set how far
//! ([`ZOOMS`]). The zoom stays while the editor's menu or type-in box shows, which shows through it. With the mouse on,
//! floating Left and Right buttons click at the cursor (held as long as the finger is, so a finger on the pad drags
//! with the button down); their grip strip on top moves them anywhere on the screen ([`lemon_card::set_buttons`]).
//! These choices and the buttons' place hold for the next editor shown full screen. The editor's `Create automation
//! clip` makes the parameter's automation track, as in the card ([`State::touch`]). The presets disc at the bottom of the
//! column opens the preset picker over it, as the card's preset button does ([`crate::plugins::presets`]); the editor
//! shows again as the picker closes.

use eframe::egui::{
    self, Area, Color32, Id, LayerId, Order, Painter, Rect, Sense, Stroke, Vec2, pos2, vec2,
};

use super::{AppCtx, Nav, Overlay};
use crate::icons::{IconId, draw_icon};
use crate::metrics::Metrics;
use crate::ui::anim;
use crate::ui::buttons::filled_circle_button;
use crate::ui::ctrl_menu::Touch;
use crate::ui::rack::PresetTarget;
use crate::ui::rack::lemon_card::{self, LemonCard};

/// The close X's disc (U): the corner logo's (`main_screen`), 2.1 U in from the screen's right and 0.2 U down, which it
/// covers.
const CLOSE: f32 = 1.5;
const CLOSE_AT: (f32, f32) = (2.1, 0.2);
/// The gap between the discs of a column (U).
const MOUSE_GAP: f32 = 0.4;
/// The zoom levels `+` and `−` step through, and the one it starts at.
const ZOOMS: [f32; 8] = [1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0];
const ZOOM: f32 = 2.0;
/// The floating mouse buttons (U): the whole pill, its grip strip on top, the gap between grip and buttons and between
/// the buttons, and their place from the screen's bottom right corner until moved.
const BUTTONS_W: f32 = 3.6;
const BUTTONS_H: f32 = 2.3;
const GRIP_H: f32 = 0.55;
const BUTTONS_GAP: f32 = 0.15;
const BUTTONS_IN: f32 = 0.4;

/// The editor shown: its texture and the pointer's.
#[derive(Default)]
pub struct State {
    card: LemonCard,
    /// The virtual mouse was turned off (it is on by default): kept for the next editor.
    direct: bool,
    /// The zoom is on, and its level (0: [`ZOOM`]), kept for the next editor.
    zoomed: bool,
    zoom: f32,
    /// Where the floating mouse buttons were moved to: their centre as a fraction of the screen (`None`: the bottom
    /// right corner), kept for the next editor.
    buttons: Option<Vec2>,
    /// The editor's `Create automation clip`, with the channel: the app takes it after the frame, as the rack's.
    pub(crate) touch: Option<(usize, Touch)>,
}

/// Shows the editor of module `module` on `channel` full screen. `Nav::Close` once it closes, or once the module or
/// its running plugin is gone.
pub fn show(
    state: &mut State,
    ui: &egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    module: i32,
) -> Option<Nav> {
    let ch = app.song.channels.get(channel);
    let plugin = (ch.and_then(|c| c.rack.get(module))).and_then(crate::ui::rack::lemon_plugin);
    let (Some(ch), Some(plugin)) = (ch, plugin) else {
        return Some(Nav::Close);
    };
    let ctx = ui.ctx().clone();
    if crate::ui::close_pressed(&ctx) {
        // The editor's own menu or type-in box goes first, then the editor.
        if !lemon_card::dismiss(&state.card) {
            return close(state);
        }
    }
    let (m, c) = (app.cx.m, &app.cx.theme.colors);
    let (id, screen, bg) = (
        Id::new("lemon full screen"),
        app.regions.screen,
        c.background.to_color32(),
    );
    crate::ui::backdrop(&ctx, id);
    let dim = ctx.layer_painter(LayerId::new(Order::Middle, id.with("backdrop")));
    dim.rect_filled(screen, 0.0, anim::dim(bg, anim::DIM_ALPHA));
    // The editor between the close X's column and its mirror on the left.
    let disc = Rect::from_min_size(
        pos2(
            screen.right() - m.u(CLOSE_AT.0),
            screen.top() + m.u(CLOSE_AT.1),
        ),
        Vec2::splat(m.u(CLOSE)),
    );
    let step = m.u(CLOSE + MOUSE_GAP);
    let mouse_disc = disc.translate(vec2(0.0, step));
    let zoom_disc = mouse_disc.translate(vec2(0.0, step));
    // The presets disc ends the column: under the zoom's, or the mouse's while the zoom's is hidden.
    let presets_disc = match state.direct {
        false => zoom_disc.translate(vec2(0.0, step)),
        true => zoom_disc,
    };
    // `+`, the level, `−`: the close X's column mirrored on the left.
    let plus_disc = Rect::from_min_size(
        pos2(screen.left() + m.u(CLOSE_AT.0 - CLOSE), disc.top()),
        disc.size(),
    );
    let level_rect = plus_disc.translate(vec2(0.0, step));
    let minus_disc = level_rect.translate(vec2(0.0, step));
    let area = screen.shrink2(vec2(m.u(CLOSE_AT.0), 0.0));
    let mouse = !state.direct;
    let level = if state.zoom > 0.0 { state.zoom } else { ZOOM };
    let zoomed = mouse && state.zoomed;
    lemon_card::set_mouse(&mut state.card, mouse);
    // The floating Left and Right buttons, with the mouse on.
    let pill = mouse.then(|| buttons_rect(screen, state.buttons, m));
    lemon_card::set_buttons(&mut state.card, pill.map(|p| button_parts(p, m)));
    // The whole screen is the virtual mouse's pad, but for the discs and the buttons.
    let mut keep_out = vec![disc, mouse_disc];
    if mouse {
        keep_out.push(zoom_disc);
    }
    keep_out.push(presets_disc);
    keep_out.extend(pill);
    if zoomed {
        keep_out.extend([plus_disc, level_rect, minus_disc]);
    }
    let keep_out = keep_out.into_iter().map(|r| r.expand(m.u(0.1))).collect();
    lemon_card::set_pad_area(&mut state.card, screen, keep_out);
    lemon_card::set_zoom(&mut state.card, if zoomed { level } else { 1.0 });
    lemon_card::set_resolution(&mut state.card, app.settings.editor_resolution());
    let rect = lemon_card::fit(area, lemon_card::shown_size(Some(&state.card), plugin));
    let (mut running, mut closed, mut toggled) = (true, false, false);
    let (mut zoom_toggled, mut zoom_step, mut presets_tapped) = (false, 0i32, false);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::hover());
            running = lemon_card::show(ui, rect, ch.id, module, plugin, &mut state.card, m);
        });
    // The discs and the floating buttons sit over the editor's own menus and type-in box (popups, at
    // `popup::ORDER`): each group on a layer of that order of its own, sized to the group (so the editor and its
    // menus still take the touches everywhere else) and raised to the top every frame, after the menus showed.
    let on_top = |name: &str, area: Rect, add: &mut dyn FnMut(&mut egui::Ui)| {
        let shown = Area::new(id.with(("on top", name)))
            .order(crate::ui::popup::ORDER)
            .fixed_pos(area.min)
            .constrain(false)
            .fade_in(false)
            .show(&ctx, |ui| {
                ui.allocate_exact_size(area.size(), Sense::hover());
                add(ui);
            });
        ctx.move_to_top(shown.response.layer_id);
    };
    // The zoom's disc only with the mouse on (the presets disc takes its place otherwise).
    let column = disc.union(mouse_disc).union(presets_disc);
    on_top("column", column, &mut |ui| {
        let ink = c.text.to_color32();
        let b = filled_circle_button(
            ui,
            id.with("presets"),
            presets_disc.center(),
            presets_disc.width(),
            bg,
        );
        draw_icon(ui.painter(), presets_disc, IconId::Music, ink, 0.6, m);
        presets_tapped = b.clicked();
        // The busy screen's abort: a `background` disc, the X in `text`.
        let x = filled_circle_button(ui, id.with("close"), disc.center(), disc.width(), bg);
        draw_icon(
            ui.painter(),
            disc,
            IconId::Close,
            c.text.to_color32(),
            0.7,
            m,
        );
        closed = x.clicked();
        // The virtual mouse's switch: lit (`accent`) while it is on.
        let on = lemon_card::mouse_on(&state.card);
        let (fill, ink) = if on {
            (c.accent.to_color32(), c.text_on_accent.to_color32())
        } else {
            (bg, c.text.to_color32())
        };
        let id = id.with("mouse");
        let b = filled_circle_button(ui, id, mouse_disc.center(), mouse_disc.width(), fill);
        mouse_icon(ui.painter(), mouse_disc, ink, m);
        toggled = b.clicked();
        if !mouse {
            return;
        }
        // The zoom's switch, lit while it is on.
        let (fill, ink) = if zoomed {
            (c.accent.to_color32(), c.text_on_accent.to_color32())
        } else {
            (bg, c.text.to_color32())
        };
        let b = filled_circle_button(
            ui,
            id.with("zoom"),
            zoom_disc.center(),
            zoom_disc.width(),
            fill,
        );
        magnifier_icon(ui.painter(), zoom_disc, ink, m);
        zoom_toggled = b.clicked();
    });
    // The Left and Right buttons: drawn only, the virtual mouse reads their touches itself (`lemon_card`).
    if let Some(pill) = pill {
        on_top("buttons", pill, &mut |ui| {
            paint_buttons(ui.painter(), pill, &state.card, &app.cx, m);
        });
    }
    // `+` and `−`, the level between them.
    if zoomed {
        let zoom_column = plus_disc.union(minus_disc);
        on_top("zoom", zoom_column, &mut |ui| {
            let ink = c.text.to_color32();
            for (r, sign, name) in [(plus_disc, 1, "zoom in"), (minus_disc, -1, "zoom out")] {
                let b = filled_circle_button(ui, id.with(name), r.center(), r.width(), bg);
                sign_icon(ui.painter(), r, sign > 0, ink, m);
                if b.clicked() {
                    zoom_step = sign;
                }
            }
            let text = format!("{level:.1}×");
            let font = m.font_id(crate::metrics::FontSize::Pt10);
            ui.painter().text(
                level_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font,
                ink,
            );
        });
    }
    // A finger on the buttons' grip moved them: kept on the screen, as a fraction of it (a turn of the screen keeps
    // them in place).
    let moved = lemon_card::take_buttons_moved(&mut state.card);
    if let Some(pill) = pill.filter(|_| moved != Vec2::ZERO) {
        let r = buttons_rect(screen, Some(fraction(screen, pill.center() + moved)), m);
        state.buttons = Some(fraction(screen, r.center()));
    }
    if toggled {
        state.direct = !state.direct;
    }
    if zoom_toggled {
        state.zoomed = !state.zoomed;
    }
    if zoom_step != 0 {
        let next = match zoom_step > 0 {
            true => ZOOMS.iter().copied().find(|&z| z > level + 1e-3),
            false => ZOOMS.iter().copied().rfind(|&z| z < level - 1e-3),
        };
        state.zoom = next.unwrap_or(level);
    }
    // The editor's `Create automation clip` is the app's (`plugins::take_automate`, issue #113), as for every
    // embedded editor.
    if lemon_card::wants_file(&mut state.card) {
        return Some(Nav::Open(Overlay::ModuleSamplePicker { channel, module }));
    }
    // The presets disc: the preset picker over the editor (hidden under it).
    if presets_tapped && !closed && running {
        let target = PresetTarget::Module(module);
        return Some(Nav::Open(Overlay::PresetPicker { channel, target }));
    }
    if closed || !running {
        return close(state);
    }
    None
}

/// Closes it, letting go of its editor's texture; the virtual mouse's choice and a touch the app has yet to take stay.
fn close(state: &mut State) -> Option<Nav> {
    *state = State {
        direct: state.direct,
        zoomed: state.zoomed,
        zoom: state.zoom,
        buttons: state.buttons,
        touch: state.touch.take(),
        ..State::default()
    };
    Some(Nav::Close)
}

/// Screen point `p` as a fraction of `screen`.
fn fraction(screen: Rect, p: egui::Pos2) -> Vec2 {
    (p - screen.min) / screen.size().max(Vec2::splat(1.0))
}

/// The floating buttons' rect: centred at `at` (a fraction of `screen`; `None`: [`BUTTONS_IN`] in from its bottom right
/// corner), kept on the screen.
fn buttons_rect(screen: Rect, at: Option<Vec2>, m: &Metrics) -> Rect {
    let size = vec2(m.u(BUTTONS_W), m.u(BUTTONS_H));
    let half = size / 2.0;
    let centre = match at {
        Some(f) => screen.min + f * screen.size(),
        None => screen.max - half - Vec2::splat(m.u(BUTTONS_IN)),
    };
    let (lo, hi) = (screen.min + half, screen.max - half);
    let centre = egui::pos2(centre.x.min(hi.x).max(lo.x), centre.y.min(hi.y).max(lo.y));
    Rect::from_center_size(centre, size)
}

/// The floating buttons' parts in `pill`: Left, Right, and the grip strip above them.
fn button_parts(pill: Rect, m: &Metrics) -> [Rect; 3] {
    let grip = pill.with_max_y(pill.top() + m.u(GRIP_H));
    let body = pill.with_min_y(grip.bottom() + m.u(BUTTONS_GAP));
    let half = (body.width() - m.u(BUTTONS_GAP)) / 2.0;
    let left = body.with_max_x(body.left() + half);
    let right = body.with_min_x(body.right() - half);
    [left, right, grip]
}

/// The floating buttons in `pill`: the grip (a `background` strip with three lines), Left and Right (`background`, lit
/// `accent` while held), each with its letter.
fn paint_buttons(
    painter: &Painter,
    pill: Rect,
    card: &LemonCard,
    cx: &crate::ui::UiCtx,
    m: &Metrics,
) {
    let c = &cx.theme.colors;
    let (bg, text) = (c.background.to_color32(), c.text.to_color32());
    let [left, right, grip] = button_parts(pill, m);
    let outline = Stroke::new(m.u(0.04).max(1.0), text.gamma_multiply(0.45));
    let corner = m.u(0.3);
    painter.rect_filled(grip, corner, bg.gamma_multiply(0.9));
    painter.rect_stroke(grip, corner, outline, egui::StrokeKind::Inside);
    let line = Stroke::new(m.u(0.05).max(1.0), text.gamma_multiply(0.7));
    for i in -1..=1 {
        let y = grip.center().y + i as f32 * m.u(0.11);
        let half = m.u(0.45);
        painter.line_segment(
            [
                egui::pos2(grip.center().x - half, y),
                egui::pos2(grip.center().x + half, y),
            ],
            line,
        );
    }
    let font = m.font_id(crate::metrics::FontSize::Pt18);
    for (r, right, label) in [(left, false, "L"), (right, true, "R")] {
        let held = lemon_card::button_held(card, right);
        let (fill, ink) = match held {
            true => (c.accent.to_color32(), c.text_on_accent.to_color32()),
            false => (bg.gamma_multiply(0.9), text),
        };
        painter.rect_filled(r, corner, fill);
        painter.rect_stroke(r, corner, outline, egui::StrokeKind::Inside);
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            ink,
        );
    }
}

/// A computer mouse in `disc` in `ink`: its body, the line between its buttons, and its wheel.
fn mouse_icon(painter: &Painter, disc: Rect, ink: Color32, m: &Metrics) {
    let size = disc.width();
    let body = Rect::from_center_size(disc.center(), vec2(size * 0.32, size * 0.48));
    let stroke = Stroke::new(m.u(0.06).max(1.0), ink);
    let round = body.width() * 0.5;
    painter.rect_stroke(body, round, stroke, egui::StrokeKind::Middle);
    let split = body.top() + body.height() * 0.42;
    let mid = body.center().x;
    painter.line_segment([pos2(mid, body.top()), pos2(mid, split)], stroke);
    painter.line_segment(
        [pos2(body.left(), split), pos2(body.right(), split)],
        stroke,
    );
    let wheel = Rect::from_center_size(
        pos2(mid, body.top() + body.height() * 0.22),
        vec2(0.0, size * 0.08),
    );
    painter.line_segment(
        [wheel.center_top(), wheel.center_bottom()],
        Stroke::new(stroke.width * 1.6, ink),
    );
}

/// A magnifier in `disc` in `ink`: its lens and its handle.
fn magnifier_icon(painter: &Painter, disc: Rect, ink: Color32, m: &Metrics) {
    let size = disc.width();
    let stroke = Stroke::new(m.u(0.07).max(1.0), ink);
    let centre = disc.center() + vec2(-0.06, -0.06) * size;
    let radius = size * 0.17;
    painter.circle_stroke(centre, radius, stroke);
    let along = vec2(1.0, 1.0).normalized();
    let from = centre + along * radius;
    let handle = Stroke::new(stroke.width * 1.6, ink);
    painter.line_segment([from, from + along * size * 0.16], handle);
}

/// A `+` (`plus`) or `−` in `disc` in `ink`.
fn sign_icon(painter: &Painter, disc: Rect, plus: bool, ink: Color32, m: &Metrics) {
    let (c, half) = (disc.center(), disc.width() * 0.2);
    let stroke = Stroke::new(m.u(0.08).max(1.0), ink);
    painter.line_segment([c - vec2(half, 0.0), c + vec2(half, 0.0)], stroke);
    if plus {
        painter.line_segment([c - vec2(0.0, half), c + vec2(0.0, half)], stroke);
    }
}
