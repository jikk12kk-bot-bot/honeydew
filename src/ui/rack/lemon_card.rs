//! A built-in Lemon plugin's card ([`crate::plugins::builtin`]). Below the header the card shows the plugin's own
//! editor, drawn by the plugin's own code (`os_plugkit::embed`) into a texture at the card's width, its aspect kept,
//! the same on every platform.
//!
//! - The pointer over it is the editor's, in the editor's pixels: presses (a second one within 0.4 s and 12 points is a
//!   double click), moves, releases (also outside, while a press it took is held) and the wheel. While the editor holds
//!   the cursor (a knob drag) the moves go as movement and the cursor hides. Issue #68: a double click on a knob or
//!   fader resets it, as a double tap does Honeydew's own. PR #87: a long press let go where it went down
//!   ([`gesture::control_menu`]) is a right click at the press, so a phone reaches a knob's menu (Reset, …) as a
//!   secondary click does on the desktop; a tap or a moved press stays a left one, and a held knob drags.
//! - The editor's menus show over the card in Lemon's own look (`os_ui::kit::menu`), with their submenus; its "Type in
//!   value" box is a text field there.
//! - Issue #52: where the editor has no file dialog (Android, the browser), the app's sample picker opens its sample
//!   ([`wants_file`]).
//! - Its edits reach the song as a plugin editor's do (`plugins::idle`): the song's params, undo, REC.
//! - Full screen it may be driven by a virtual mouse ([`set_mouse`]): the screen turns into a touchpad that moves an
//!   arrow cursor over the editor, for the editors' small controls, which a fingertip covers ([`trackpad`]). The cursor
//!   drives the editor's menus and type-in box too, and a zoom about the cursor magnifies the editor ([`set_zoom`]).

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Area, Color32, ColorImage, CursorIcon, Event, Id, Key, LayerId, Modifiers,
    MouseWheelUnit, Order, PointerButton, Pos2, Rect, Response, Sense, TextEdit, TextureHandle,
    TextureOptions, Ui, Vec2, pos2, vec2,
};
use os_plugkit::embed::{self, Answer, Button, Input, MenuView, Modal};
use os_ui::Canvas;
use os_ui::kit::menu::{self as kmenu, MenuLayout};

use crate::metrics::Metrics;
use crate::plugins;
use crate::plugins::builtin::Builtin;
use crate::ui::{Finger, MOUSE, fingers, focus_lost, gesture};

/// Two presses closer than this (seconds and points) are a double click: a finger's second tap lands a little off.
const DOUBLE_S: f64 = 0.4;
const DOUBLE_PT: f32 = 12.0;
/// The editor's polling period while it shows and is busy (its own timer on the desktop), and while it rests: busy is
/// within `BUSY_S` of a painted frame, or with the pointer on it. A parameter change wakes it at once.
// ponytail: a resting editor polls at 4 Hz (its overlays expire a little late); a wake from the editor itself would let
// it sleep.
const TICK: std::time::Duration = std::time::Duration::from_millis(30);
const REST: std::time::Duration = std::time::Duration::from_millis(250);
const BUSY_S: f64 = 0.5;
/// Points per wheel notch for a wheel that scrolls by points.
const WHEEL_POINTS: f32 = 50.0;
/// The type-in box's width in the editor's pixels at scale 1, as Lemon's.
const TYPE_IN_W: f32 = 140.0;
/// The virtual mouse: cursor points per finger point, and its arrow's height in U.
const PAD_SPEED: f32 = 1.25;
const ARROW_U: f32 = 0.55;
/// How much larger the zoom paints the editor at most: each frame is painted by the CPU, and its cost grows with the
/// square.
const SHARP_ZOOM: f32 = 2.0;

/// The virtual mouse's state ([`trackpad`]).
#[derive(Clone, Debug, Default)]
struct Pad {
    /// Where a finger drives it ([`set_pad_area`]; `None`: the editor), and the rects inside that are not the pad (the
    /// buttons over it).
    area: Option<Rect>,
    keep_out: Vec<Rect>,
    /// The zoom about the cursor ([`set_zoom`]): 1 (or less) is none.
    zoom: f32,
    /// A menu or type-in box showed last frame.
    modal: bool,
    /// The virtual mouse is on: its cursor's position as a fraction of the editor's size (a resize keeps it).
    cursor: Option<Vec2>,
    /// The finger on the pad: its id, where it went down and when; and where it was last.
    down: Option<(u64, Pos2, f64)>,
    last: Pos2,
    /// The other fingers down on the pad meanwhile.
    others: Vec<u64>,
    /// It left its calm zone: the finger moves the cursor.
    moved: bool,
    /// The floating mouse buttons ([`set_buttons`]): Left's, Right's and their grip's rects (points).
    buttons: Option<[Rect; 3]>,
    /// The finger holding each (Left, Right, the grip), and whether Left's and Right's press reached the editor (one
    /// that opened a menu is the menu's).
    held: [Option<u64>; 3],
    sent: [bool; 2],
    /// The grip's finger was last here, and how far it moved the buttons since [`take_buttons_moved`].
    grip_last: Pos2,
    grip_moved: Vec2,
}

/// A Lemon card's own state: its editor's texture and the pointer's.
#[derive(Default)]
pub(crate) struct LemonCard {
    /// The editor shown ([`plugins::lemon_editor`]).
    editor: embed::Parent,
    texture: Option<TextureHandle>,
    /// The frame shown, its size in pixels and when it came.
    version: u64,
    painted: f64,
    size: (u32, u32),
    /// The button of the press the editor took: its moves and release are the editor's wherever they are.
    pressed: Option<Button>,
    /// That press is a finger's other than the pointer's ([`other_finger`]): its id and the pass it was last seen in.
    finger: Option<(u64, u64)>,
    /// The pointer's last position, and whether it was over the editor.
    last: Pos2,
    over: bool,
    /// The last press: when and where, for a double click.
    click: Option<(f64, Pos2)>,
    menu: Option<MenuUi>,
    /// The type-in box's text, while it is open.
    typed: Option<String>,
    /// The press that opened the type-in box (a text field opens it at a press) lasts: its release is not a click away.
    typing_press: bool,
    /// The editor asks for a file ([`Modal::OpenFile`]), and the request went to the app ([`wants_file`]) or is about to.
    asked_file: bool,
    wants_file: bool,
    /// The virtual mouse ([`set_mouse`]).
    pad: Pad,
    /// The share of the screen's pixels it paints at ([`set_resolution`]); 0 (unset) is all of them.
    resolution: f32,
}

impl std::fmt::Debug for LemonCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LemonCard")
            .field("editor", &self.editor)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The height of a card body showing an editor of `size` at `width`.
pub(super) fn body_height(size: (u32, u32), width: f32) -> f32 {
    width * size.1 as f32 / size.0.max(1) as f32
}

/// Issue #68: the largest rect of `size`'s aspect inside `area`, centred in it: where full screen shows the editor.
pub(crate) fn fit(area: Rect, size: (u32, u32)) -> Rect {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let k = (area.width() / w).min(area.height() / h);
    Rect::from_center_size(area.center(), vec2(w, h) * k)
}

/// The size whose aspect the card shows `plugin`'s editor at: its last frame's, as an editor may resize itself (Tesla,
/// from its grip or its saved size), else the plugin's own.
pub(crate) fn shown_size(card: Option<&LemonCard>, plugin: &Builtin) -> (u32, u32) {
    (card.map(|c| c.size))
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or_else(|| plugin.size())
}

/// Draws module `instance_id`'s editor (built-in plugin `plugin`, on the channel with id `channel_id`) in the card body
/// `content` (points). `false` while the plugin is not running: the caller shows the plugin card instead.
pub(crate) fn show(
    ui: &mut Ui,
    content: Rect,
    channel_id: i32,
    instance_id: i32,
    plugin: &Builtin,
    card: &mut LemonCard,
    m: &Metrics,
) -> bool {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    // The editor at the card's width in device pixels, so that it is sharp ([`plugins::lemon_editor`] opens it again
    // only when that moves by more than a little), up to the largest texture (full screen on a big screen).
    // The virtual mouse's zoom paints it larger (up to [`SHARP_ZOOM`]), so that the part shown is sharper.
    let (w, h) = plugin.size();
    let most = ctx.input(|i| i.max_texture_side) as f32 / w.max(h).max(1) as f32;
    let zoom = zoom_of(card).min(SHARP_ZOOM);
    // Full screen may paint at less than the screen's pixels (`Plugin editor resolution:`): faster, softer.
    let resolution = match card.resolution {
        r if r > 0.0 => r.min(1.0),
        _ => 1.0,
    };
    let scale = (content.width() * zoom * ppp * resolution / w as f32)
        .max(0.5)
        .min(most);
    // The handle's type is the platform's (`embed::Parent`: `usize` on macOS).
    let Some(editor) = plugins::lemon_editor(channel_id, instance_id, scale)
        .map(|id| -> embed::Parent { id as _ })
    else {
        return false;
    };
    if card.editor != editor {
        // A reopened editor keeps the virtual mouse: where its cursor was, its pad, its zoom.
        let pad = std::mem::take(&mut card.pad);
        *card = LemonCard {
            editor,
            resolution: card.resolution,
            ..LemonCard::default()
        };
        card.pad = Pad {
            down: None,
            others: Vec::new(),
            modal: false,
            held: [None; 3],
            sent: [false; 2],
            ..pad
        };
    }
    let size = shown_size(Some(card), plugin);
    let rect = Rect::from_min_size(
        content.min,
        vec2(content.width(), body_height(size, content.width())),
    );
    // Global (an editor shows in one card at a time): tests and render harnesses find the card by its editor.
    let id = Id::new(("lemon editor", editor));
    let mut response = ui.interact(rect, id, Sense::click_and_drag());
    // Editor pixels per point.
    let k = card.size.0 as f32 / rect.width();
    let modal = embed::modal(editor);
    // A file request waits for the app's picker: the virtual mouse goes on driving the editor meanwhile.
    let menu_shows = matches!(modal, Some(Modal::Menu { .. } | Modal::TypeIn { .. }));
    // The virtual mouse drives the editor, and while a menu or type-in box shows, that (`pad`: its cursor and click).
    let mut pad = None;
    if card.pad.cursor.is_some() && card.size.0 > 0 {
        if menu_shows && !card.pad.modal {
            // The press that asked is the modal's (its answer delivers it again, with its release): the finger still
            // down only moves the cursor now.
            (card.pressed, card.finger) = (None, None);
            let p = &mut card.pad;
            p.sent = [false; 2];
            p.moved |= p.down.is_some();
        }
        card.pad.modal = menu_shows;
        let click = trackpad(ui, card, rect, k, m, !menu_shows);
        // Where the cursor shows on the screen: through the zoom, as the menus and type-in box do.
        let lens = Lens {
            rect,
            view: view(card, rect),
        };
        pad = Some(PadPointer {
            pos: lens.show(rect.min + card.pad.cursor.unwrap_or_default() * rect.size()),
            click,
        });
    } else if modal.is_none() && card.size.0 > 0 {
        other_finger(ui, card, &response, rect, k);
        // The pointer's left press the editor holds, before this frame's release lets go of it.
        let left = card.pressed == Some(Button::Left) && card.finger.is_none();
        forward(ui, card, rect, k, response.hovered());
        // PR #87: a long press on the editor let go where it went down is a right click there (a phone has no secondary
        // button), as a knob's or slider's of Honeydew's own opens its menu: held, the press drags the knob at once. The
        // left press has had its release; a right click that opens no menu has its own.
        if let Some(origin) = gesture::control_menu(ui, &mut response, m).filter(|_| left) {
            let (x, y) = ((origin.x - rect.left()) * k, (origin.y - rect.top()) * k);
            let held = mods(ui.input(|i| i.modifiers));
            let btn = Button::Right;
            embed::input(
                editor,
                Input::Down {
                    x,
                    y,
                    btn,
                    dbl: false,
                },
                held,
            );
            embed::pump(editor);
            if embed::modal(editor).is_none() {
                embed::input(editor, Input::Up { btn }, held);
            }
        }
    } else {
        // The press that asked is the modal's (its answer delivers it again, with its release): the next press is new.
        (card.pressed, card.finger) = (None, None);
    }
    embed::pump(editor);
    // Issue #72: only what changed goes into the texture (a whole frame is megabytes, a knob turn a small patch of
    // it); all of it into a new texture or at a new size.
    let (shown, whole) = (card.size, card.texture.is_none());
    let mut fresh = None;
    if let Some(v) = embed::frame(editor, card.version, |f| {
        let size = (f.width, f.height);
        let area = (whole || size != shown).then_some((0, 0, size.0, size.1));
        fresh = (area.or(f.changed)).map(|a| (size, a, image(&f.pixels, f.width, a)));
    }) {
        card.version = v;
        card.painted = ctx.input(|i| i.time);
    }
    if let Some((size, (x, y, ..), image)) = fresh {
        card.size = size;
        match &mut card.texture {
            Some(t) if size == shown => {
                t.set_partial([x as usize, y as usize], image, TextureOptions::LINEAR)
            }
            Some(t) => t.set(image, TextureOptions::LINEAR),
            None => {
                card.texture = Some(ctx.load_texture(
                    format!("lemon editor {editor}"),
                    image,
                    TextureOptions::LINEAR,
                ))
            }
        }
    }
    // Zoomed, the part of the editor about the cursor fills the rect; it stays zoomed while a menu or type-in box shows,
    // which shows through the same zoom.
    let view = view(card, rect);
    let lens = Lens { rect, view };
    paint_view(ui, rect, view, card);
    let locked = card.pressed.is_some() && embed::locked(editor);
    if locked {
        ctx.set_cursor_icon(CursorIcon::None);
    }
    // The virtual mouse's arrow, over everything (the editor's menus too), hidden while a knob or fader holds the
    // cursor (as the desktop's is).
    if let Some(cursor) = card.pad.cursor
        && !locked
    {
        let tip = lens.show(rect.min + cursor * rect.size());
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, id.with("cursor")));
        arrow(&painter, tip, m);
    }
    match modal {
        Some(Modal::Menu { x, y, menu, scale }) => {
            show_menu(&ctx, id, card, lens, k, (x, y), menu, scale, pad)
        }
        Some(Modal::TypeIn { x, y, text, scale }) => {
            type_in(&ctx, id, card, lens, k, (x, y), text, scale, pad)
        }
        Some(Modal::OpenFile) => {
            card.wants_file |= !std::mem::replace(&mut card.asked_file, true);
            // A new press on the editor (with the virtual mouse, anywhere): the picker went without answering.
            let on = response.hovered() || card.pad.cursor.is_some();
            if on && ctx.input(|i| i.pointer.any_pressed()) {
                embed::answer(editor, None);
            }
        }
        None => (card.menu, card.typed, card.asked_file) = (None, None, false),
    }
    let now = ctx.input(|i| i.time);
    let busy =
        now - card.painted < BUSY_S || over(card) || card.menu.is_some() || card.typed.is_some();
    ctx.request_repaint_after(if busy { TICK } else { REST });
    true
}

/// Paints the editor's last frame in `rect`, as it was: the card's while its editor shows full screen.
pub(crate) fn paint(ui: &Ui, rect: Rect, card: &LemonCard) {
    paint_view(ui, rect, rect, card);
}

/// Paints the part `view` (in `rect`'s points) of the editor's last frame over all of `rect`. A view standing out of
/// the editor (a menu out of it shows) paints the editor's part only, where the zoom puts it.
fn paint_view(ui: &Ui, rect: Rect, view: Rect, card: &LemonCard) {
    let Some(t) = &card.texture else {
        return;
    };
    let shown = view.intersect(rect);
    if !(shown.width() > 0.0 && shown.height() > 0.0) {
        return;
    }
    let lens = Lens { rect, view };
    let size = rect.size().max(Vec2::splat(1e-3));
    let uv = Rect::from_min_max(
        ((shown.min - rect.min) / size).to_pos2(),
        ((shown.max - rect.min) / size).to_pos2(),
    );
    let at = Rect::from_min_max(lens.show(shown.min), lens.show(shown.max));
    ui.painter().image(t.id(), at, uv, Color32::WHITE);
}

/// How the editor shows through the virtual mouse's zoom: the part `view` of the editor's points fills `rect` (the
/// editor's place on the screen). Not zoomed, `view` is `rect` and points show where they are.
#[derive(Clone, Copy, Debug)]
struct Lens {
    rect: Rect,
    view: Rect,
}

impl Lens {
    /// Screen points per editor point.
    fn z(&self) -> f32 {
        self.rect.width() / self.view.width().max(1e-3)
    }

    /// Where the editor's point `p` (unzoomed) shows on the screen.
    fn show(&self, p: Pos2) -> Pos2 {
        self.rect.min + (p - self.view.min) * self.z()
    }
}

/// The virtual mouse's zoom as it shows: 1 while the mouse is off.
fn zoom_of(card: &LemonCard) -> f32 {
    match card.pad.cursor {
        Some(_) => card.pad.zoom.max(1.0),
        None => 1.0,
    }
}

/// The part of the editor in `rect` (its points) that shows in `rect`: all of it, or zoomed in ([`set_zoom`]) the part
/// that much smaller centred on the cursor, kept inside the editor. It stays zoomed while a menu shows, and may then
/// go as far as the menu stands out of the editor, so that all of it can be reached.
fn view(card: &LemonCard, rect: Rect) -> Rect {
    let (zoom, Some(cursor)) = (zoom_of(card), card.pad.cursor) else {
        return rect;
    };
    if zoom <= 1.0 {
        return rect;
    }
    let size = rect.size() / zoom;
    let centre = rect.min + cursor * rect.size();
    let k = card.size.0 as f32 / rect.width().max(1e-3);
    let limit = (card.menu.iter())
        .flat_map(|m| m.levels.iter().map(move |l| (l, m.scale)))
        .filter(|_| k > 0.0)
        .map(|(l, scale)| Rect::from_min_size(rect.min + l.at / k, l.size(scale) / k))
        .fold(rect, |limit, r| limit.union(r));
    let min = (centre - size / 2.0).clamp(limit.min, limit.max - size);
    Rect::from_min_size(min, size)
}

/// Cancels the editor's open menu or type-in box, as Escape over it would: whether one was open. The app's `close`
/// takes Escape (and Back) first.
pub(crate) fn dismiss(card: &LemonCard) -> bool {
    let open = matches!(
        embed::modal(card.editor),
        Some(Modal::Menu { .. } | Modal::TypeIn { .. })
    );
    if open {
        embed::answer(card.editor, None);
    }
    open
}

/// Whether the editor asked for a file to open since the last call: the app's sample picker answers it.
pub(crate) fn wants_file(card: &mut LemonCard) -> bool {
    std::mem::take(&mut card.wants_file)
}

/// Turns the virtual mouse on or off: on, the editor is a touchpad with a cursor on it ([`trackpad`]), starting at its
/// centre; off, touches press where they land. A press the virtual mouse held is released.
pub(crate) fn set_mouse(card: &mut LemonCard, on: bool) {
    if on == card.pad.cursor.is_some() {
        return;
    }
    let editor = card.editor;
    let send = |e: Input| embed::input(editor, e, (false, false, false));
    if card.pad.sent[1] {
        send(Input::Up { btn: Button::Right });
    }
    if let Some(btn) = card.pressed.take() {
        send(Input::Up { btn });
    }
    if std::mem::take(&mut card.over) {
        send(Input::Leave);
    }
    card.finger = None;
    card.pad = Pad {
        cursor: on.then_some(vec2(0.5, 0.5)),
        ..Pad::default()
    };
}

/// Whether the virtual mouse is on.
pub(crate) fn mouse_on(card: &LemonCard) -> bool {
    card.pad.cursor.is_some()
}

/// Where a finger drives the virtual mouse: anywhere in `area` (full screen: the whole screen) but on `keep_out` (the
/// buttons over it). Until set, the editor itself.
pub(crate) fn set_pad_area(card: &mut LemonCard, area: Rect, keep_out: Vec<Rect>) {
    (card.pad.area, card.pad.keep_out) = (Some(area), keep_out);
}

/// The virtual mouse's floating buttons, Left and Right, and the grip that moves them (rects in points; `None`: none).
/// A finger on Left or Right holds that button at the cursor; on the grip it moves them ([`take_buttons_moved`]). They
/// should be in the pad's `keep_out` too ([`set_pad_area`]).
pub(crate) fn set_buttons(card: &mut LemonCard, buttons: Option<[Rect; 3]>) {
    card.pad.buttons = buttons;
}

/// How far a finger on the floating buttons' grip moved them since the last call (points).
pub(crate) fn take_buttons_moved(card: &mut LemonCard) -> Vec2 {
    std::mem::take(&mut card.pad.grip_moved)
}

/// Whether the floating button `right` (else Left) is held.
pub(crate) fn button_held(card: &LemonCard, right: bool) -> bool {
    card.pad.held[usize::from(right)].is_some()
}

/// The virtual mouse's zoom: the editor shows `zoom` times larger about its cursor (1: not zoomed). The editor paints
/// again at that size, to be sharp.
pub(crate) fn set_zoom(card: &mut LemonCard, zoom: f32) {
    card.pad.zoom = zoom.max(1.0);
}

/// Paints the editor at `resolution` (0 to 1) of the screen's pixels: full screen's `Plugin editor resolution:`. A
/// change opens the editor again at the new size.
pub(crate) fn set_resolution(card: &mut LemonCard, resolution: f32) {
    card.resolution = resolution.clamp(0.1, 1.0);
}

/// Whether the pointer was over a Lemon editor this frame: the rack's wheel is then the editor's.
pub(super) fn over(card: &LemonCard) -> bool {
    card.over || card.pressed.is_some()
}

/// The pixels (x0, y0, x1, y1) of XRGB rows of `w` as an image.
fn image(px: &[u32], w: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> ColorImage {
    let size = [(x1 - x0) as usize, (y1 - y0) as usize];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for row in px
        .chunks_exact(w.max(1) as usize)
        .take(y1 as usize)
        .skip(y0 as usize)
    {
        let rgb = |&p: &u32| Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
        pixels.extend(row[x0 as usize..x1 as usize].iter().map(rgb));
    }
    ColorImage::new(size, pixels)
}

fn button(b: PointerButton) -> Option<Button> {
    match b {
        PointerButton::Primary => Some(Button::Left),
        PointerButton::Secondary => Some(Button::Right),
        PointerButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

fn mods(m: Modifiers) -> (bool, bool, bool) {
    (m.ctrl || m.mac_cmd, m.shift, m.alt)
}

/// Issue #68: a finger other than the pointer's (egui's pointer follows the first finger down only) presses, moves and
/// lifts as the pointer does while no press holds the editor, so that a knob turns while another finger plays keys; the
/// pointer waits meanwhile ([`forward`]). A focus loss or a frame without the card lifts it.
fn other_finger(ui: &Ui, card: &mut LemonCard, response: &Response, rect: Rect, k: f32) {
    let (editor, pass) = (card.editor, ui.ctx().cumulative_pass_nr());
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let send = |e: Input| embed::input(editor, e, (false, false, false));
    let held = |card: &LemonCard, id| card.finger.is_some_and(|f| f.0 == id);
    if card
        .finger
        .is_some_and(|f| f.1 + 1 != pass || ui.input(focus_lost))
    {
        send(Input::Up { btn: Button::Left });
        (card.pressed, card.finger) = (None, None);
    }
    // The pointer's own finger, which [`forward`] takes.
    let pointer = ui.input(|i| i.pointer.primary_pressed()) && response.is_pointer_button_down_on();
    for f in fingers(ui, response.id) {
        match f {
            Finger::Down(id, at) if id != MOUSE && !pointer && card.pressed.is_none() => {
                let (x, y) = px(at);
                send(Input::Down {
                    x,
                    y,
                    btn: Button::Left,
                    dbl: false,
                });
                (card.pressed, card.finger, card.last) = (Some(Button::Left), Some((id, pass)), at);
            }
            Finger::Move(id, at) if held(card, id) => {
                let d = (at - card.last) * k;
                let (x, y) = px(at);
                let locked = embed::locked(editor);
                send(if locked {
                    Input::Delta { dx: d.x, dy: d.y }
                } else {
                    Input::Move { x, y }
                });
                card.last = at;
            }
            Finger::Up(id, _) if held(card, id) => {
                send(Input::Up { btn: Button::Left });
                (card.pressed, card.finger) = (None, None);
            }
            _ => {}
        }
    }
    if let Some(f) = card.finger.as_mut() {
        f.1 = pass;
    }
}

/// This frame's pointer input to the editor shown in `rect` (`k` editor pixels per point); `hovered`: the pointer is
/// on it, with nothing over it. Nothing while another finger holds the editor ([`other_finger`]).
fn forward(ui: &Ui, card: &mut LemonCard, rect: Rect, k: f32, hovered: bool) {
    let editor = card.editor;
    if card.finger.is_some() {
        return;
    }
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let (events, now, held) = ui.input(|i| (i.events.clone(), i.time, i.modifiers));
    let send = |e: Input, m: Modifiers| embed::input(editor, e, mods(m));
    for event in events {
        match event {
            Event::PointerMoved(p) => {
                if card.pressed.is_some() && embed::locked(editor) {
                    let d = (p - card.last) * k;
                    send(Input::Delta { dx: d.x, dy: d.y }, held);
                } else if card.pressed.is_some() || (hovered && rect.contains(p)) {
                    let (x, y) = px(p);
                    send(Input::Move { x, y }, held);
                    card.over = true;
                } else if std::mem::take(&mut card.over) {
                    send(Input::Leave, held);
                }
                card.last = p;
            }
            Event::PointerButton {
                pos,
                button: b,
                pressed: true,
                modifiers,
            } if card.pressed.is_none() && hovered && rect.contains(pos) => {
                let Some(btn) = button(b) else { continue };
                let dbl = (card.click)
                    .is_some_and(|(t, at)| now - t < DOUBLE_S && at.distance(pos) < DOUBLE_PT);
                card.click = (!dbl).then_some((now, pos));
                let (x, y) = px(pos);
                send(Input::Down { x, y, btn, dbl }, modifiers);
                // Issue #68: a double press that starts a drag (a knob or fader holds the cursor) resets it first, by
                // a middle click (every Lemon editor's reset; most ignore `dbl`), and the drag goes on from there.
                if dbl && btn == Button::Left {
                    embed::pump(editor);
                    if embed::locked(editor) {
                        let down = |btn| Input::Down {
                            x,
                            y,
                            btn,
                            dbl: false,
                        };
                        let middle = Button::Middle;
                        let reset = [
                            Input::Up { btn },
                            down(middle),
                            Input::Up { btn: middle },
                            down(btn),
                        ];
                        reset.into_iter().for_each(|e| send(e, modifiers));
                    }
                }
                (card.pressed, card.last, card.over) = (Some(btn), pos, true);
            }
            Event::PointerButton {
                button: b,
                pressed: false,
                modifiers,
                ..
            } if card.pressed.is_some() && card.pressed == button(b) => {
                let btn = card.pressed.take().expect("held");
                send(Input::Up { btn }, modifiers);
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
            } if hovered && rect.contains(card.last) => {
                let notches = match unit {
                    MouseWheelUnit::Point => delta.y / WHEEL_POINTS,
                    MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
                };
                let (x, y) = px(card.last);
                send(Input::Wheel { x, y, notches }, modifiers);
            }
            Event::PointerGone if card.pressed.is_none() && std::mem::take(&mut card.over) => {
                send(Input::Leave, held);
            }
            _ => {}
        }
    }
}

/// The virtual mouse's click while a menu or type-in box shows ([`trackpad`]): its cursor (points), and whether a
/// floating button clicked there this frame.
#[derive(Clone, Copy, Debug)]
struct PadPointer {
    pos: Pos2,
    click: bool,
}

/// The virtual mouse's frame: a finger anywhere on its area ([`set_pad_area`]) moves its cursor over the editor in
/// `rect` (`k` editor pixels per point), [`Pad::cursor`]. `deliver`: the editor takes it (no menu or type-in box
/// shows); else the cursor moves over the screen and a floating button's press is a click there, which it returns,
/// for the menu.
///
/// - A finger on the pad only moves the cursor (by [`PAD_SPEED`] times the finger's move, less when zoomed), hovering
///   what it passes over. It never clicks: no tap, long press, double tap or tap and press does anything.
/// - Only the floating buttons click ([`set_buttons`]): Left and Right press and release their button at the cursor,
///   held as long as the finger is (so another finger on the pad drags with the button down: a knob turns, or the
///   cursor moves with the button held); over a menu or type-in box either is a click. A finger on their grip moves
///   them ([`take_buttons_moved`]).
/// - The wheel turns at the cursor.
///
/// The steps are read from the frame's own events, each finger by its id (a touch, or the mouse), in order: a button
/// press and lift in the same frame still clicks, and a finger on a button does not disturb the one on the pad.
fn trackpad(ui: &Ui, card: &mut LemonCard, rect: Rect, k: f32, m: &Metrics, deliver: bool) -> bool {
    use eframe::egui::TouchPhase;
    let editor = card.editor;
    let Some(fraction) = card.pad.cursor else {
        return false;
    };
    let mut clicked = false;
    let mut at = rect.min + fraction * rect.size();
    // Back on the editor once a menu that let it out is gone.
    if deliver {
        at = at.clamp(rect.min, rect.max);
    }
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let (held, now, none_down, lost, events) = ui.input(|i| {
        (
            i.modifiers,
            i.time,
            !i.pointer.any_down() && !i.any_touches(),
            focus_lost(i),
            i.events.clone(),
        )
    });
    let send = |e: Input| embed::input(editor, e, mods(held));
    // Every touch is a finger of its own; the mouse's primary button one more, in frames without touches (egui's
    // integrations repeat the first touch as the pointer).
    let touches = events.iter().any(|e| matches!(e, Event::Touch { .. }));
    let steps: Vec<Finger> = (events.iter())
        .filter_map(|e| match *e {
            Event::Touch { id, phase, pos, .. } => Some(match phase {
                TouchPhase::Start => Finger::Down(id.0, pos),
                TouchPhase::Move => Finger::Move(id.0, pos),
                TouchPhase::End | TouchPhase::Cancel => Finger::Up(id.0, pos),
            }),
            _ if touches => None,
            Event::PointerMoved(pos) => Some(Finger::Move(MOUSE, pos)),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                ..
            } => Some(match pressed {
                true => Finger::Down(MOUSE, pos),
                false => Finger::Up(MOUSE, pos),
            }),
            _ => None,
        })
        .collect();
    let area = card.pad.area.unwrap_or(rect);
    let keep_out = card.pad.keep_out.clone();
    let buttons = card.pad.buttons;
    let on_pad = |o: Pos2| area.contains(o) && !keep_out.iter().any(|r| r.contains(o));
    // Zoomed in, the finger moves the cursor less: the editor under it moves as far as the finger on the screen.
    let speed = PAD_SPEED * view(card, rect).width() / rect.width().max(1e-3);
    // The cursor stays on the editor, or while a menu shows, on the pad (a menu may stand out of the editor).
    let bounds = if deliver { rect } else { area.union(rect) };
    // The pad's finger at `p`: the cursor follows once it leaves its calm zone.
    let move_to = |card: &mut LemonCard, at: &mut Pos2, p: Pos2| {
        let Some((_, origin, _)) = card.pad.down else {
            return;
        };
        let pad = &mut card.pad;
        if !pad.moved && gesture::left_calm(origin, Some(p), m.u(0.4)) {
            pad.moved = true;
        }
        let d = p - pad.last;
        pad.last = p;
        if !pad.moved || d == Vec2::ZERO {
            return;
        }
        if card.pressed.is_some() && embed::locked(editor) {
            send(Input::Delta {
                dx: d.x * speed * k,
                dy: d.y * speed * k,
            });
        } else {
            *at = (*at + d * speed).clamp(bounds.min, bounds.max);
            if deliver {
                let (x, y) = px(*at);
                send(Input::Move { x, y });
                card.over = true;
            }
        }
    };
    // The pad's finger lifts: it clicks nothing.
    let release = |card: &mut LemonCard| {
        card.pad.down = None;
        card.pad.others.clear();
    };
    for step in steps {
        match step {
            Finger::Down(f, p) => {
                // The floating buttons first: Left, Right, then their grip.
                let hit = buttons.and_then(|b| b.iter().position(|r| r.contains(p)));
                if let Some(b) = hit {
                    if card.pad.held[b].is_some() {
                        continue;
                    }
                    card.pad.held[b] = Some(f);
                    if b == 2 {
                        card.pad.grip_last = p;
                        continue;
                    }
                    let btn = if b == 0 { Button::Left } else { Button::Right };
                    if !deliver {
                        clicked = true;
                    } else if !(btn == Button::Left && card.pressed.is_some()) {
                        let (x, y) = px(at);
                        send(Input::Down {
                            x,
                            y,
                            btn,
                            dbl: false,
                        });
                        card.pad.sent[b] = true;
                        if btn == Button::Left {
                            card.pressed = Some(Button::Left);
                        }
                    }
                    continue;
                }
                if !on_pad(p) {
                    continue;
                }
                if card.pad.down.is_some() {
                    // A second finger on the pad: the first goes on moving the cursor.
                    card.pad.others.push(f);
                    continue;
                }
                let pad = &mut card.pad;
                (pad.down, pad.last, pad.moved) = (Some((f, p, now)), p, false);
            }
            Finger::Move(f, p) => {
                if card.pad.held[2] == Some(f) {
                    card.pad.grip_moved += p - card.pad.grip_last;
                    card.pad.grip_last = p;
                } else if card.pad.down.is_some_and(|d| d.0 == f) {
                    move_to(card, &mut at, p);
                }
            }
            Finger::Up(f, p) => {
                if let Some(b) = card.pad.held.iter().position(|&h| h == Some(f)) {
                    card.pad.held[b] = None;
                    if b == 2 {
                        card.pad.grip_moved += p - card.pad.grip_last;
                    } else if std::mem::take(&mut card.pad.sent[b]) {
                        let btn = if b == 0 { Button::Left } else { Button::Right };
                        send(Input::Up { btn });
                        if btn == Button::Left {
                            card.pressed = None;
                        }
                    }
                } else if card.pad.down.is_some_and(|d| d.0 == f) {
                    move_to(card, &mut at, p);
                    release(card);
                } else {
                    card.pad.others.retain(|&o| o != f);
                }
            }
        }
    }
    // A lift that never came (the focus went, or a frame without the card took it): every finger is let go.
    if lost || none_down {
        if card.pad.down.is_some() {
            release(card);
        }
        for b in 0..3 {
            card.pad.held[b] = None;
            if b < 2 && std::mem::take(&mut card.pad.sent[b]) {
                let btn = if b == 0 { Button::Left } else { Button::Right };
                send(Input::Up { btn });
                if btn == Button::Left {
                    card.pressed = None;
                }
            }
        }
        card.pad.others.clear();
    }
    for event in events {
        if let Event::MouseWheel {
            unit,
            delta,
            modifiers,
        } = event
            && deliver
        {
            let notches = match unit {
                MouseWheelUnit::Point => delta.y / WHEEL_POINTS,
                MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
            };
            let (x, y) = px(at);
            embed::input(editor, Input::Wheel { x, y, notches }, mods(modifiers));
        }
    }
    if rect.width() > 0.0 && rect.height() > 0.0 {
        card.pad.cursor = Some((at - rect.min) / rect.size());
    }
    clicked
}

/// The virtual mouse's arrow, its tip at `tip`: white, outlined in black, [`ARROW_U`] tall.
fn arrow(painter: &egui::Painter, tip: Pos2, m: &Metrics) {
    use eframe::egui::{Shape, Stroke, epaint::PathShape};
    // The classic arrow on a grid 19 tall: its outline, and the convex pieces that fill it.
    const OUTLINE: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 16.0),
        (4.0, 12.5),
        (7.0, 19.0),
        (9.5, 18.0),
        (6.5, 11.5),
        (11.5, 11.5),
    ];
    let s = m.u(ARROW_U) / 19.0;
    let at = |&(x, y): &(f32, f32)| tip + vec2(x, y) * s;
    let pieces: [&[(f32, f32)]; 4] = [
        &[(0.0, 0.0), (0.0, 16.0), (4.0, 12.5)],
        &[(0.0, 0.0), (4.0, 12.5), (6.5, 11.5)],
        &[(0.0, 0.0), (6.5, 11.5), (11.5, 11.5)],
        &[(4.0, 12.5), (7.0, 19.0), (9.5, 18.0), (6.5, 11.5)],
    ];
    for piece in pieces {
        let points = piece.iter().map(at).collect();
        // A hairline of the fill closes the seams between the pieces.
        let seam = Stroke::new(0.5, Color32::WHITE);
        painter.add(Shape::convex_polygon(points, Color32::WHITE, seam));
    }
    let points = OUTLINE.iter().map(at).collect();
    let stroke = Stroke::new((1.5 * s).max(1.0), Color32::BLACK);
    painter.add(PathShape::closed_line(points, stroke));
}

/// One level of an open menu: the menu, its layout, its top left in editor pixels, the row under the pointer, and its
/// painted image.
struct Level {
    menu: MenuView,
    layout: MenuLayout,
    at: Vec2,
    hover: Option<usize>,
    texture: Option<(TextureHandle, Option<usize>)>,
}

impl Level {
    fn new(menu: MenuView, at: Vec2, scale: f32) -> Option<Level> {
        let mut measure = Canvas::new(8, 8, scale).ok()?;
        let layout = kmenu::layout(&mut measure, &menu.items, menu.style);
        Some(Level {
            menu,
            layout,
            at,
            hover: None,
            texture: None,
        })
    }

    /// Its size in editor pixels.
    fn size(&self, scale: f32) -> Vec2 {
        vec2(
            (self.layout.w * scale).ceil(),
            (self.layout.h * scale).ceil(),
        )
    }

    /// The clickable row at editor pixel `p`.
    fn row_at(&self, p: Vec2, scale: f32) -> Option<usize> {
        let q = (p - self.at) / scale;
        let row = self.layout.item_at(&self.menu.items, q.x, q.y)?;
        self.menu.items[row].clickable().then_some(row)
    }
}

/// An open menu: the modal it shows, its levels (the menu, then each submenu open), and when it opened.
struct MenuUi {
    shown: (i32, i32, MenuView),
    levels: Vec<Level>,
    /// The editor's scale it shows at (editor pixels per menu unit).
    scale: f32,
    opened: f64,
    moved: bool,
}

/// Shows the editor's menu `menu` at editor pixel `at` over the editor in `rect`, and answers it: a row picked, or
/// cancelled by a press outside or Escape.
#[allow(clippy::too_many_arguments)]
fn show_menu(
    ctx: &egui::Context,
    id: Id,
    card: &mut LemonCard,
    lens: Lens,
    k: f32,
    at: (i32, i32),
    menu: MenuView,
    scale: f32,
    pad: Option<PadPointer>,
) {
    let editor = card.editor;
    let now = ctx.input(|i| i.time);
    let key = (at.0, at.1, menu.clone());
    if card.menu.as_ref().is_none_or(|m| m.shown != key) {
        let Some(root) = Level::new(menu, vec2(at.0 as f32, at.1 as f32), scale) else {
            return embed::answer(editor, None);
        };
        card.menu = Some(MenuUi {
            shown: key,
            levels: vec![root],
            scale,
            opened: now,
            moved: false,
        });
    }
    let m = card.menu.as_mut().expect("just set");
    m.scale = scale;
    let screen = ctx.content_rect();
    let rect = lens.rect;
    // A level's rect in points, through the zoom, kept on the screen.
    let place = |l: &Level| {
        let min = rect.min + l.at / k;
        let r = Rect::from_min_max(lens.show(min), lens.show(min + l.size(scale) / k));
        let min = r.min.min(screen.max - r.size()).max(screen.min);
        Rect::from_min_size(min, r.size())
    };
    // A finger's lift takes the pointer away in its own frame: where it was is where it lifted. With the virtual mouse,
    // its cursor is the pointer and its taps the clicks (each a new one: it may pick at once).
    let pointer = match pad {
        Some(pad) => Some(pad.pos),
        None => ctx.input(|i| i.pointer.interact_pos()),
    };
    // Editor pixels per screen point.
    let kz = k / lens.z();
    let editor_px = |p: Pos2, l: &Level| l.at + (p - place(l).min) * kz;
    if let Some(p) = pointer {
        m.moved |= pad.is_some() || ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        // The deepest level under the pointer takes the hover; a row with a submenu opens it.
        if let Some(depth) = (0..m.levels.len())
            .rev()
            .find(|&d| place(&m.levels[d]).contains(p))
        {
            let hover = m.levels[depth].row_at(editor_px(p, &m.levels[depth]), scale);
            m.levels[depth].hover = hover;
            let sub = hover.and_then(|r| m.levels[depth].menu.sub(r).cloned().map(|s| (r, s)));
            let open = m.levels.get(depth + 1).map(|l| l.menu.clone());
            match sub {
                Some((row, s)) if open.as_ref() != Some(&s) => {
                    m.levels.truncate(depth + 1);
                    let r = m.levels[depth].layout.rows[row];
                    let at =
                        m.levels[depth].at + vec2((r.x + r.w) * scale - 2.0, r.y * scale - 2.0);
                    m.levels.extend(Level::new(s, at, scale));
                }
                Some(_) => {}
                None => m.levels.truncate(depth + 1),
            }
        }
    }
    // Picks and cancels.
    let (pressed, released, escape) = ctx.input_mut(|i| {
        let escape = i.consume_key(Modifiers::NONE, Key::Escape);
        (i.pointer.any_pressed(), i.pointer.any_released(), escape)
    });
    let (pressed, released) = match pad {
        Some(pad) => (pad.click, pad.click),
        None => (pressed, released),
    };
    let inside = pointer.is_some_and(|p| m.levels.iter().any(|l| place(l).contains(p)));
    if escape || (pressed && !inside) {
        card.menu = None;
        return embed::answer(editor, None);
    }
    // The release of the press that opened it does not pick.
    if released && inside && (m.moved || now - m.opened > DOUBLE_S) {
        let p = pointer.expect("inside");
        if let Some(depth) = (0..m.levels.len())
            .rev()
            .find(|&d| place(&m.levels[d]).contains(p))
        {
            let l = &m.levels[depth];
            if let Some(row) = l
                .row_at(editor_px(p, l), scale)
                .filter(|&r| l.menu.sub(r).is_none())
            {
                let mut path: Vec<usize> =
                    (m.levels[..depth].iter()).filter_map(|l| l.hover).collect();
                path.push(row);
                card.menu = None;
                return embed::answer(editor, Some(Answer::Pick(path)));
            }
        }
    }
    let palette = os_plugkit::palette();
    for (d, l) in m.levels.iter_mut().enumerate() {
        let painted = l.texture.as_ref().map(|(_, h)| *h);
        if painted != Some(l.hover) {
            let size = l.size(scale);
            let Ok(mut canvas) = Canvas::new(size.x as u32, size.y as u32, scale) else {
                continue;
            };
            canvas.begin_frame(None);
            let (items, layout, style) = (&l.menu.items, &l.layout, l.menu.style);
            kmenu::paint(
                &mut canvas,
                &palette,
                items,
                layout,
                style,
                0.0,
                0.0,
                l.hover,
                l.menu.current,
            );
            let mut px = vec![0u32; (size.x * size.y) as usize];
            canvas.copy_to_xrgb(&mut px);
            let img = image(&px, size.x as u32, (0, 0, size.x as u32, size.y as u32));
            match &mut l.texture {
                Some((t, h)) => {
                    t.set(img, TextureOptions::LINEAR);
                    *h = l.hover;
                }
                None => {
                    l.texture = Some((
                        ctx.load_texture(format!("lemon menu {d}"), img, TextureOptions::LINEAR),
                        l.hover,
                    ))
                }
            }
        }
        let r = place(l);
        if let Some((t, _)) = &l.texture {
            // Above the full-screen editor (a `Foreground` area), which a press on it (a tap moving the virtual
            // mouse) would otherwise raise over the menu.
            Area::new(id.with(("menu", d)))
                .order(crate::ui::popup::ORDER)
                .fixed_pos(r.min)
                .show(ctx, |ui| {
                    ui.allocate_rect(r, Sense::click_and_drag());
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    ui.painter().image(t.id(), r, uv, Color32::WHITE);
                });
        }
    }
}

/// Shows the editor's "Type in value" box at editor pixel `at` over the editor in `rect`, starting with `text`: Enter
/// or its `OK` answers with what was typed, Escape or a press outside cancels.
#[allow(clippy::too_many_arguments)]
fn type_in(
    ctx: &egui::Context,
    id: Id,
    card: &mut LemonCard,
    lens: Lens,
    k: f32,
    at: (i32, i32),
    text: String,
    scale: f32,
    pad: Option<PadPointer>,
) {
    let editor = card.editor;
    let fresh = card.typed.is_none();
    let down = ctx.input(|i| i.pointer.any_down());
    let opening = card.typing_press;
    card.typing_press = (fresh || opening) && down;
    let typed = card.typed.get_or_insert(text);
    // Through the zoom, as wide as it shows on the editor (within the screen).
    let screen = ctx.content_rect();
    let width = (TYPE_IN_W * scale / k * lens.z()).min(screen.width() * 0.9);
    let min = lens.show(lens.rect.min + vec2(at.0 as f32, at.1 as f32) / k);
    let field = id.with("type in");
    let (mut done, mut cancel) = (false, false);
    let shown = Area::new(id.with("type in area"))
        .order(crate::ui::popup::ORDER)
        .fixed_pos(min)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut r = ui.add(TextEdit::singleline(typed).id(field).desired_width(width));
                // The whole text starts selected: typing replaces it. The release of the press that opened it keeps
                // it.
                if fresh || (opening && r.lost_focus()) {
                    r.request_focus();
                    let mut state = TextEditState::load(ui.ctx(), field).unwrap_or_default();
                    let all =
                        CCursorRange::two(CCursor::new(0), CCursor::new(typed.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), field);
                }
                // Issue #91: Android has no keyboard for an egui field; its text editor types the value (the Lemon
                // Slicer's BPM and BEATS have no other way in on a phone), and its OK is this field's Enter.
                crate::android_text::edit(&mut r, typed, usize::MAX);
                // Issue #166: an OK beside it, so a touch never depends on a soft keyboard's Enter reaching egui.
                // Twice as wide as tall, for a finger.
                let h = r.rect.height();
                let ok = ui.add(egui::Button::new("OK").min_size(vec2(2.0 * h, h)));
                let ok = ok.clicked();
                done = ok || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                let escape = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                // With the virtual mouse a finger anywhere moves its cursor: the box keeps the focus, and only a
                // click of the cursor outside it (or Escape) cancels.
                if pad.is_some() && r.lost_focus() && !done && !escape {
                    r.request_focus();
                }
                cancel = escape || (pad.is_none() && r.lost_focus() && !done && !fresh && !opening);
            });
        })
        .response
        .rect;
    let outside = match pad {
        Some(pad) => pad.click && !shown.contains(pad.pos),
        None => ctx.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.contains(p))
        }),
    };
    if done {
        let text = card.typed.take().unwrap_or_default();
        embed::answer(editor, Some(Answer::Text(text)));
    } else if cancel || outside {
        card.typed = None;
        embed::answer(editor, None);
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{CentralPanel, Frame, RawInput, TouchDeviceId, TouchId, TouchPhase};
    use lemon_sampler::view;

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::plugins::builtin::named;
    use crate::song::{ChannelKind, Song};

    /// A finger as Android reports it (egui-winit): the touch, with the pointer's move and press; its moves; its lift,
    /// with the release and the pointer gone.
    fn finger(phase: TouchPhase, pos: Pos2) -> Vec<Event> {
        let touch = Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        match phase {
            TouchPhase::Start => vec![touch, Event::PointerMoved(pos), button(true)],
            TouchPhase::Move => vec![touch, Event::PointerMoved(pos)],
            _ => vec![touch, button(false), Event::PointerGone],
        }
    }

    /// A Lemon Sampler's card (or `name`'s) at the screen's top left, one editor pixel a point.
    struct Rig {
        name: &'static str,
        /// The screen it fills, as full screen does ([`fit`]); `None`: one editor pixel a point at the top left.
        full: Option<Rect>,
        ctx: egui::Context,
        card: LemonCard,
        /// Its channel's id and its module's instance id.
        ids: (i32, i32),
        time: f64,
    }

    impl Rig {
        fn new(name: &'static str) -> Rig {
            let mut song = Song::default();
            song.add_channel(ChannelKind::Instrument);
            let channel = &mut song.channels[1];
            let id = channel.id;
            let module = plugins::add(&mut channel.rack, id, named(name).uid, 0).unwrap();
            plugins::sync(&song);
            let mut rig = Rig {
                name,
                full: None,
                ctx: egui::Context::default(),
                card: LemonCard::default(),
                ids: (id, module),
                time: 0.0,
            };
            // The editor opens, then paints.
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let (card, (channel, module), name) = (&mut self.card, self.ids, self.name);
            let full = self.full;
            let _ = self.ctx.run(input, |ctx| {
                CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
                    let sampler = named(name);
                    let width = sampler.size().0 as f32;
                    let content = match full {
                        Some(screen) => fit(screen, shown_size(Some(card), sampler)),
                        None => Rect::from_min_size(Pos2::ZERO, vec2(width, 500.0)),
                    };
                    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
                    assert!(show(ui, content, channel, module, sampler, card, &m));
                });
            });
        }

        fn tap(&mut self, at: Pos2) {
            self.frame(finger(TouchPhase::Start, at));
            self.frame(finger(TouchPhase::End, at));
        }

        /// The centre of row `row` of the menu shown (at scale 1, on the screen as it is).
        fn row(&self, row: usize) -> Pos2 {
            let level = &self.card.menu.as_ref().expect("a menu shows").levels[0];
            let r = level.layout.rows[row];
            Pos2::ZERO + level.at + vec2(r.cx(), r.cy())
        }
    }

    /// The centre of the sampler's view rect `r` (`view::rr`) in its editor's pixels.
    fn centre(r: os_ui::Rect) -> Pos2 {
        pos2(r.cx() - view::CROP_X as f32, r.cy() - view::CROP_Y as f32)
    }

    /// Issue #68: a double tap on a knob (Lemon Chorus's Delay tile) puts it back at its default, the second tap a few
    /// points off the first.
    #[test]
    fn a_double_tap_resets_a_knob() {
        let mut rig = Rig::new("Lemon Chorus");
        // The tile (`lemon_small_fx::chorus`: x 8…166, y 8…54).
        let at = pos2(87.0, 31.0);
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, at));
        let default = (plugins::idle().iter().find(|m| m.gesture == 0))
            .expect("a press")
            .value;
        for y in [20.0, 0.0] {
            rig.frame(finger(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, pos2(at.x, 0.0)));
        let last =
            |moves: &[plugins::PluginMove]| moves.iter().rfind(|m| m.gesture == 1).map(|m| m.value);
        let moves = plugins::idle();
        assert!(last(&moves).is_some_and(|v| v != default), "{moves:?}");
        rig.tap(at);
        rig.tap(at + vec2(6.0, 3.0));
        rig.frame(vec![]);
        assert_eq!(last(&plugins::idle()), Some(default));
    }

    /// A second finger, as Android reports one: the touch alone.
    fn second(phase: TouchPhase, pos: Pos2) -> Vec<Event> {
        vec![Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(2),
            phase,
            pos,
            force: None,
        }]
    }

    /// The virtual mouse: a drag moves its cursor by the finger's move (and turns no knob); a tap, a double tap, a tap
    /// then a drag and a long press on the pad click nothing; the floating Left button held while another finger drags
    /// on the pad turns the knob under the cursor.
    #[test]
    fn the_virtual_mouse_drives_the_editor_at_its_cursor() {
        let mut rig = Rig::new("Lemon Chorus");
        set_mouse(&mut rig.card, true);
        assert!(mouse_on(&rig.card));
        // The whole screen is the pad, as full screen's, but for the floating buttons.
        let left = Rect::from_min_size(pos2(700.0, 500.0), vec2(40.0, 40.0));
        let right = Rect::from_min_size(pos2(745.0, 500.0), vec2(40.0, 40.0));
        let grip = Rect::from_min_size(pos2(700.0, 545.0), vec2(85.0, 10.0));
        let pill = left.union(right).union(grip);
        set_pad_area(
            &mut rig.card,
            Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
            vec![pill],
        );
        set_buttons(&mut rig.card, Some([left, right, grip]));
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(rig.card.size.0 as f32, 500.0));
        let rect = Rect::from_min_size(
            rect.min,
            vec2(rect.width(), body_height(rig.card.size, rect.width())),
        );
        let cursor = |rig: &Rig| rect.min + rig.card.pad.cursor.expect("on") * rect.size();
        // The cursor onto the Delay tile (`lemon_small_fx::chorus`: x 8…166, y 8…54), the finger far from it.
        let tile = pos2(87.0, 31.0);
        let from = pos2(300.0, 150.0);
        let _ = plugins::idle();
        let start = cursor(&rig);
        let to = from + (tile - start) / PAD_SPEED;
        rig.frame(finger(TouchPhase::Start, from));
        for k in 1..=4 {
            rig.frame(finger(
                TouchPhase::Move,
                from + (to - from) * k as f32 / 4.0,
            ));
        }
        rig.frame(finger(TouchPhase::End, to));
        assert!(cursor(&rig).distance(tile) < 1.0, "{:?}", cursor(&rig));
        assert!(plugins::idle().is_empty(), "a move turns nothing");
        let (at, fraction) = (cursor(&rig), rig.card.pad.cursor);
        // A tap, a double tap, a tap then a press that drags up, a long press: nothing.
        rig.tap(from);
        rig.tap(from);
        rig.tap(from);
        rig.frame(finger(TouchPhase::Start, from));
        for y in [130.0, 100.0, 60.0] {
            rig.frame(finger(TouchPhase::Move, pos2(from.x, y)));
        }
        rig.frame(finger(TouchPhase::End, pos2(from.x, 60.0)));
        rig.frame(finger(TouchPhase::Start, from));
        for _ in 0..12 {
            rig.frame(finger(TouchPhase::Move, from));
        }
        rig.frame(finger(TouchPhase::End, from));
        rig.frame(vec![]);
        assert!(plugins::idle().is_empty(), "the pad clicks nothing");
        assert_eq!(embed::modal(rig.card.editor), None, "no menu");
        // Back onto the tile (the drag up moved the cursor), then Left held and a second finger dragging up: the knob
        // turns.
        rig.card.pad.cursor = fraction;
        assert!(cursor(&rig).distance(at) < 1.0, "{:?}", cursor(&rig));
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, left.center()));
        rig.frame(second(TouchPhase::Start, from));
        for y in [130.0, 100.0, 60.0] {
            rig.frame(second(TouchPhase::Move, pos2(from.x, y)));
        }
        rig.frame(second(TouchPhase::End, pos2(from.x, 60.0)));
        rig.frame(finger(TouchPhase::End, left.center()));
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 1),
            "the knob turned"
        );
        set_mouse(&mut rig.card, false);
        assert!(!mouse_on(&rig.card));
    }

    /// The virtual mouse's tap clicks nothing, even when its press and lift come in the same frame; the floating Left
    /// button clicks at the cursor, its press and lift in one frame too.
    #[test]
    fn only_the_left_button_clicks_at_the_cursor() {
        let mut rig = Rig::new("Lemon Sampler");
        let editor = rig.card.editor;
        set_mouse(&mut rig.card, true);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let left = Rect::from_min_size(pos2(700.0, 500.0), vec2(40.0, 40.0));
        let right = Rect::from_min_size(pos2(745.0, 500.0), vec2(40.0, 40.0));
        let grip = Rect::from_min_size(pos2(700.0, 545.0), vec2(85.0, 10.0));
        let pill = left.union(right).union(grip);
        set_pad_area(&mut rig.card, screen, vec![pill]);
        set_buttons(&mut rig.card, Some([left, right, grip]));
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(rig.card.size.0 as f32, 500.0));
        let rect = Rect::from_min_size(
            rect.min,
            vec2(rect.width(), body_height(rig.card.size, rect.width())),
        );
        let cursor = |rig: &Rig| rect.min + rig.card.pad.cursor.expect("on") * rect.size();
        // Time stretching's `Mode` list (`view::sample_tab`): the cursor onto it.
        let mode = centre(view::rr(456.0, 548.0, 102.0, 122.0));
        let from = pos2(300.0, 150.0);
        let to = from + (mode - cursor(&rig)) / PAD_SPEED;
        rig.frame(finger(TouchPhase::Start, from));
        for k in 1..=4 {
            rig.frame(finger(
                TouchPhase::Move,
                from + (to - from) * k as f32 / 4.0,
            ));
        }
        rig.frame(finger(TouchPhase::End, to));
        assert!(cursor(&rig).distance(mode) < 1.0, "{:?}", cursor(&rig));
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        // A tap on the pad, a quick one (press and lift in one frame) and a double tap: no click.
        rig.tap(from);
        let mut quick = finger(TouchPhase::Start, from);
        quick.extend(finger(TouchPhase::End, from));
        rig.frame(quick);
        rig.tap(from);
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        assert_eq!(embed::modal(editor), None, "the pad clicked");
        // The Left button, pressed and lifted in one frame.
        let mut quick = finger(TouchPhase::Start, left.center());
        quick.extend(finger(TouchPhase::End, left.center()));
        rig.frame(quick);
        rig.frame(vec![]);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "Left clicked"
        );
        assert!(dismiss(&rig.card));
        rig.frame(vec![]);
        // The grip moves the buttons.
        rig.frame(finger(TouchPhase::Start, grip.center()));
        rig.frame(finger(TouchPhase::Move, grip.center() - vec2(100.0, 50.0)));
        rig.frame(finger(TouchPhase::End, grip.center() - vec2(100.0, 50.0)));
        assert_eq!(take_buttons_moved(&mut rig.card), vec2(-100.0, -50.0));
        assert!(
            cursor(&rig).distance(mode) < 1.0,
            "the grip moves no cursor"
        );
    }

    /// Issue #68: full screen, the editor fills the screen as far as its aspect allows, centred, painted at that scale
    /// (sharp, not stretched); a drag on a knob there turns it and a double tap puts it back, where the knob shows.
    #[test]
    fn full_screen_fits_the_editor_and_maps_the_finger() {
        let mut rig = Rig::new("Lemon Chorus");
        let size = named("Lemon Chorus").size();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let rect = fit(screen, size);
        let aspect = size.0 as f32 / size.1 as f32;
        assert!((rect.aspect_ratio() - aspect).abs() < 1e-3, "{rect:?}");
        assert!(screen.expand(1e-3).contains_rect(rect), "{rect:?}");
        assert!(rect.width() == screen.width() || rect.height() == screen.height());
        assert!(
            (rect.center() - screen.center()).length() < 1e-3,
            "{rect:?}"
        );
        // Opened again at the new scale.
        rig.full = Some(screen);
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            (rig.card.size.0 as f32 - rect.width()).abs() <= 1.0,
            "{:?}",
            rig.card.size
        );
        // The Delay tile (`lemon_small_fx::chorus`: x 8…166, y 8…54) where it shows.
        let at = rect.min + vec2(87.0, 31.0) * (rect.width() / size.0 as f32);
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, at));
        let default = (plugins::idle().iter().find(|m| m.gesture == 0))
            .expect("a press")
            .value;
        for dy in [10.0, 40.0] {
            rig.frame(finger(TouchPhase::Move, at - vec2(0.0, dy)));
        }
        rig.frame(finger(TouchPhase::End, at - vec2(0.0, 40.0)));
        let last =
            |moves: &[plugins::PluginMove]| moves.iter().rfind(|m| m.gesture == 1).map(|m| m.value);
        let moves = plugins::idle();
        assert!(last(&moves).is_some_and(|v| v != default), "{moves:?}");
        rig.tap(at);
        rig.tap(at + vec2(6.0, 3.0));
        rig.frame(vec![]);
        assert_eq!(last(&plugins::idle()), Some(default));
    }

    /// Issue #68: a second finger turns a knob while the first (egui's pointer) holds a key elsewhere, which then lifts
    /// and moves nothing in the editor.
    #[test]
    fn a_second_finger_turns_a_knob() {
        let mut rig = Rig::new("Lemon Chorus");
        let (at, key) = (pos2(87.0, 31.0), pos2(790.0, 590.0));
        let second = |phase, pos| {
            vec![Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(2),
                phase,
                pos,
                force: None,
            }]
        };
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, key));
        rig.frame(second(TouchPhase::Start, at));
        for y in [20.0, 0.0] {
            rig.frame(second(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, key));
        let moves = plugins::idle();
        let values = moves.iter().filter(|m| m.gesture == 1).count();
        assert!(
            values > 0 && !moves.iter().any(|m| m.gesture == 2),
            "{moves:?}"
        );
        rig.frame(second(TouchPhase::End, pos2(at.x, 0.0)));
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 2),
            "the lift ends the press"
        );
    }

    /// Issue #52: on a phone, a list's menu takes the row tapped, and the editor the next tap.
    #[test]
    fn a_finger_picks_a_list_row_and_taps_on() {
        let mut rig = Rig::new("Lemon Sampler");
        let editor = rig.card.editor;
        // Time stretching's `Mode` list (`view::sample_tab`).
        let mode = centre(view::rr(456.0, 548.0, 102.0, 122.0));
        rig.tap(mode);
        assert!(matches!(embed::modal(editor), Some(Modal::Menu { .. })));
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        let level = &rig.card.menu.as_ref().expect("the card shows it").levels[0];
        let row = (0..level.menu.items.len())
            .find(|&r| level.menu.items[r].clickable() && Some(r) != level.menu.current)
            .expect("another mode");
        let _ = plugins::idle();
        rig.tap(rig.row(row));
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "the tap picked the row");
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 1),
            "the pick set the mode"
        );
        rig.tap(mode);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "the next tap is the editor's"
        );
    }

    /// PR #87: a long press on a knob let go in place is a right click: the editor's menu (Reset, …) opens at the knob,
    /// and a quick tap does not. A press outside cancels; a press that moves is a drag, never a menu.
    #[test]
    fn a_long_press_on_a_knob_opens_its_menu_but_a_tap_does_not() {
        let mut rig = Rig::new("Lemon Sampler");
        let editor = rig.card.editor;
        // The sample page's `START OFFSET` knob, in editor pixels (the card shows the view cropped).
        let knob = pos2(
            45.0 - view::CROP_X as f32,
            view::y(286.0) - view::CROP_Y as f32,
        );
        rig.tap(knob);
        rig.frame(vec![]);
        assert!(embed::modal(editor).is_none(), "a tap opens no menu");
        // Past the double tap's 0.4 s (a second press within it resets the knob instead).
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        // A long press: down, then 0.7 s of frames with the finger unmoved: no menu while held, one on the lift.
        rig.frame(finger(TouchPhase::Start, knob));
        for _ in 0..7 {
            rig.frame(vec![]);
        }
        assert!(embed::modal(editor).is_none(), "no menu while held");
        rig.frame(finger(TouchPhase::End, knob));
        rig.frame(vec![]);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "the lift of the long press opened the knob's menu"
        );
        assert!(rig.card.menu.is_some(), "the card shows it");
        rig.frame(vec![]);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "it stays open"
        );
        rig.tap(pos2(700.0, 550.0));
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "the outside press cancelled");
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        rig.frame(finger(TouchPhase::Start, knob));
        rig.frame(finger(TouchPhase::Move, knob + vec2(30.0, 0.0)));
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        rig.frame(finger(TouchPhase::End, knob + vec2(30.0, 0.0)));
        assert!(embed::modal(editor).is_none(), "a moved press is a drag");
    }

    /// A text field the editor opens a type-in for at a press (Lemon Formula Controller's
    /// formula) keeps it open past that press's release, for the finger's typing.
    #[test]
    fn a_text_fields_type_in_outlasts_the_tap_that_opened_it() {
        let mut rig = Rig::new("Lemon Formula Controller");
        let editor = rig.card.editor;
        // The formula field (`lemon_small_fx::formula_controller`: x 192…467, y 58…81), held a few frames.
        let at = pos2(330.0, 70.0);
        rig.frame(finger(TouchPhase::Start, at));
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig.frame(finger(TouchPhase::End, at));
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(matches!(embed::modal(editor), Some(Modal::TypeIn { .. })));
        // It has the keys: a formula, then Enter.
        let enter = Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        rig.frame(vec![Event::Text("t".into())]);
        rig.frame(vec![enter]);
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "Enter answered it");
        // The editor took it: the next type-in starts from it.
        rig.frame(finger(TouchPhase::Start, at));
        rig.frame(vec![]);
        let typed = matches!(embed::modal(editor), Some(Modal::TypeIn { text, .. }) if text == "t");
        assert!(typed, "{:?}", embed::modal(editor));
        // Issue #166: its OK, at the box's right end, answers it too, with no Enter from the keyboard.
        rig.frame(finger(TouchPhase::End, at));
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        rig.frame(vec![Event::Text("u".into())]);
        let area = Id::new(("lemon editor", editor)).with("type in area");
        let shown = rig
            .ctx
            .memory(|m| m.area_rect(area))
            .expect("the box shows");
        rig.tap(shown.right_center() - vec2(8.0, 0.0));
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "OK answered it");
        rig.frame(finger(TouchPhase::Start, at));
        rig.frame(vec![]);
        let typed = matches!(embed::modal(editor), Some(Modal::TypeIn { text, .. }) if text == "u");
        assert!(typed, "{:?}", embed::modal(editor));
    }
}
