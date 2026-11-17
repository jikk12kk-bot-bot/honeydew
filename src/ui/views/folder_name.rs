//! The track folder dialog, a folder row's `Rename Folder` / `Folder Color`: the title and the close X over the
//! `Folder Name` field, inline, with `Enter Folder Name` as its hint, and the `Folder Color` slider (its hue). A
//! floating dialog centred in the content area over the screen dimmed to α 0.6, as `Channel Settings`'
//! ([`super::channel_settings`]).

use eframe::egui::{self, Align, Align2, Area, Id, Key, Margin, Order, Rect, Sense, TextEdit, vec2};

use super::{AppCtx, Nav};
use crate::metrics::FontSize;
use crate::theme::channel_color;
use crate::ui::widgets::{Control, ParamMenu, h_slider};

/// The box, in U.
const SIZE: [f32; 2] = [10.0, 5.75];
/// Inset of the rows from the box's sides.
const PAD: f32 = 0.5;
/// Rows from the box's top: (top, height), in U.
const TITLE: (f32, f32) = (0.25, 1.0);
const CAPTION: (f32, f32) = (1.5, 0.5);
const FIELD: (f32, f32) = (2.0, 1.25);
const SLIDER: (f32, f32) = (3.5, 2.0);
/// The close X's cell at the top-right corner.
const CLOSE: f32 = 1.5;

/// Shows folder `folder`'s name dialog, modal, `name` holding its field's text. The name applies on Enter, which closes
/// the dialog, or on a click outside or the close X, which close it too; a blank one is ignored, and Escape closes it
/// without the name. The colour applies as the slider moves. Param edits (the audio never reads them). A gone folder
/// closes it.
pub fn show(ui: &mut egui::Ui, app: &mut AppCtx, folder: usize, name: &mut String) -> Option<Nav> {
    if folder >= app.song.file.folders.len() {
        return Some(Nav::Close);
    }
    let ctx = ui.ctx().clone();
    let (theme, m) = (app.cx.theme, app.cx.m);
    let c = &theme.colors;
    let (text, bg) = (c.text.to_color32(), c.background.to_color32());
    let screen = app.regions.screen;
    let area = app.regions.area;
    let width = m.u(SIZE[0]).min(screen.width() - 4.0 / m.ppp());
    let rect = Rect::from_center_size(area.center(), vec2(width, m.u(SIZE[1])));
    let row = |(top, h): (f32, f32)| {
        Rect::from_x_y_ranges(
            rect.left() + m.u(PAD)..=rect.right() - m.u(PAD),
            rect.top() + m.u(top)..=rect.top() + m.u(top + h),
        )
    };
    let id = Id::new("folder_name");
    let backdrop = crate::ui::dialog::dialog_box(&ctx, id, (c, m), screen, rect);
    let escape = crate::ui::close_pressed(&ctx);
    let mut close = backdrop.clicked() || backdrop.secondary_clicked() || escape;
    let mut hue = app.song.file.folders[folder].hue;
    // The dialog's first frame.
    let seen = id.with("seen");
    let first = ctx.data_mut(|d| !std::mem::replace(d.get_temp_mut_or(seen, false), true));
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let painter = ui.painter().clone();
            let title = Rect::from_x_y_ranges(rect.x_range(), row(TITLE).y_range());
            let font = m.font_id(FontSize::Pt12);
            painter.text(title.center(), Align2::CENTER_CENTER, "Folder Settings", font, text);
            let caption = row(CAPTION);
            let font = m.font_id(FontSize::Pt8);
            painter.text(caption.left_center(), Align2::LEFT_CENTER, "Folder Name", font, text);

            let field = row(FIELD);
            painter.rect_filled(field, 0.0, bg);
            let visuals = ui.visuals_mut();
            visuals.text_cursor.stroke.color = text;
            visuals.selection.bg_fill = c.accent.to_color32().gamma_multiply(0.5);
            visuals.selection.stroke.color = text;
            let edit = TextEdit::singleline(&mut *name)
                .id(id.with("name"))
                .hint_text("Enter Folder Name")
                .char_limit(crate::song::folders::MAX_NAME)
                .frame(false)
                .margin(Margin::symmetric(m.u(0.25) as i8, 0))
                .font(m.font_id(FontSize::Pt10))
                .text_color(text)
                .desired_width(f32::INFINITY)
                .min_size(field.size())
                .vertical_align(Align::Center);
            let mut response = ui.put(field, edit);
            crate::android_text::edit(&mut response, name, crate::song::folders::MAX_NAME);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                close = true;
            } else if first
                || (crate::android_text::REFOCUS && ui.memory(|mem| mem.focused().is_none()))
            {
                // Focused as it opens, so the keyboard (Android's text editor) comes up at once; after that a phone's
                // field is tapped to rename again (as `Channel Settings`').
                response.request_focus();
            }
            let ink = channel_color(hue, theme).to_color32();
            let control = Control {
                menu: ParamMenu::Off,
                bipolar: false,
                ..Control::new(id.with("color"), row(SLIDER), "Folder Color", ink, hue)
            };
            h_slider(ui, &app.cx, &control, &mut hue, |_| String::new());
            let x = Rect::from_min_size(
                rect.right_top() - vec2(m.u(CLOSE), 0.0),
                vec2(m.u(CLOSE), m.u(CLOSE)),
            );
            let surface = (c.menu_fill.to_color32(), 0.375);
            close |= crate::ui::buttons::close_x(ui, &app.cx, id.with("close"), x, surface, 0.6)
                .clicked();
        });
    let mut edited = app.song.set_folder_hue(folder, hue);
    edited |= close && !escape && app.song.rename_folder(folder, name);
    if edited {
        app.param_edited();
    }
    if close {
        ctx.memory_mut(|mem| mem.surrender_focus(id.with("name")));
        ctx.data_mut(|d| d.remove::<bool>(seen));
    }
    close.then_some(Nav::Close)
}
