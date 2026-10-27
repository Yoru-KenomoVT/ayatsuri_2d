// The one shell every floating dialog uses, so they all match Settings and
// About: frameless, no outline, a title strip that moves the window, the
// same close button, the same padding. Positions live in egui's memory per
// title, so a dialog reopens where it was left. A big window (Texture Atlas,
// Render, Timelapse) asks for `resizable` and gets a corner to drag; its
// content then fills whatever size it is given.

use egui::{Align2, FontId, Id, Margin, Pos2, Rect, Sense, Ui, Vec2, vec2};

use crate::theme;

const HEADER: f32 = 40.0;

/// Every dialog notes the frame it was drawn in, so a window that should
/// not land on top of another (the update offer) can wait its turn.
pub(crate) fn mark_shown(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new("dialog-shown"), frame));
}

/// A dialog was drawn this frame or the one before.
pub(crate) fn dialog_shown_lately(ctx: &egui::Context) -> bool {
    let last = ctx.data(|d| d.get_temp::<u64>(Id::new("dialog-shown")));
    last.is_some_and(|f| f + 1 >= ctx.cumulative_frame_nr())
}

pub(crate) struct Dialog<'a> {
    title: String,
    open: Option<&'a mut bool>,
    width: f32,
    at: Option<Pos2>,
    /// Default and smallest size, for a dialog the user can resize.
    resize: Option<(Vec2, Vec2)>,
}

impl<'a> Dialog<'a> {
    pub(crate) fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), open: None, width: 340.0, at: None, resize: None }
    }

    /// Shows the close button and Esc, both clearing the flag.
    pub(crate) fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }

    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// A corner to resize it by, starting at `size` and never smaller than
    /// `min`; the content fills the window.
    pub(crate) fn resizable(mut self, size: Vec2, min: Vec2) -> Self {
        self.resize = Some((size, min));
        self.width = size.x;
        self
    }

    /// Where the dialog's centre starts, instead of the window centre.
    pub(crate) fn at(mut self, centre: Pos2) -> Self {
        self.at = Some(centre);
        self
    }

    pub(crate) fn show<R>(self, ctx: &egui::Context, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let Dialog { title, open, width, at, resize } = self;
        if open.as_deref() == Some(&false) {
            return None;
        }
        mark_shown(ctx);
        let id = Id::new(("dialog", title.as_str()));
        let pos_id = id.with("pos");
        let start = at.unwrap_or(ctx.content_rect().center());
        // a resizable dialog is pinned by its top-left corner, so dragging
        // its resize corner grows it there instead of about its centre
        let start = match resize {
            Some((size, _)) => start - size / 2.0,
            None => start,
        };
        let pos = ctx.data(|d| d.get_temp::<Pos2>(pos_id)).unwrap_or(start);
        let style = ctx.style_of(ctx.theme());
        let mut close = false;
        let mut window = egui::Window::new(title.as_str())
            .id(id)
            .title_bar(false)
            // only the header strip moves it; egui's own drag-anywhere
            // fought the pinned position and snapped back
            .movable(false)
            .collapsible(false)
            .frame(egui::Frame::window(&style).inner_margin(0.0).stroke(egui::Stroke::NONE))
            .current_pos(pos);
        window = match resize {
            Some((size, min)) => {
                window.resizable(true).default_size(size).min_size(min).pivot(Align2::LEFT_TOP)
            }
            None => window.resizable(false).pivot(Align2::CENTER_CENTER),
        };
        let out = window.show(ctx, |ui| {
            let width = if resize.is_some() { ui.available_width() } else { width };
            ui.set_width(width);
            let (reserved, _) = ui.allocate_exact_size(vec2(width, HEADER), Sense::hover());
            let inner = egui::Frame::NONE
                .inner_margin(Margin { left: 18, right: 18, top: 4, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(width - 36.0);
                    if resize.is_some() {
                        ui.set_min_height(ui.available_height());
                    }
                    add(ui)
                })
                .inner;
            // the header spans the window as it ended up, so content wider
            // than asked for still has its close button in the corner
            let strip = Rect::from_min_max(
                reserved.min,
                egui::pos2(ui.min_rect().right().max(reserved.right()), reserved.bottom()),
            );
            // the strip is the handle; the close button sits on top
            let drag = ui.interact(strip, id.with("drag"), Sense::drag());
            if drag.dragged() {
                let moved = pos + drag.drag_delta();
                ctx.data_mut(|d| d.insert_temp(pos_id, moved));
            }
            ui.painter().text(
                strip.left_center() + vec2(18.0, 2.0),
                Align2::LEFT_CENTER,
                title.as_str(),
                FontId::proportional(14.5),
                theme::pal().text,
            );
            if open.is_some() {
                let x = Rect::from_center_size(
                    strip.right_center() + vec2(-20.0, 2.0),
                    vec2(24.0, 24.0),
                );
                let resp = ui.interact(x, id.with("close"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(x, 4.0, theme::pal().select);
                }
                ui.painter().text(
                    x.center(),
                    Align2::CENTER_CENTER,
                    egui_phosphor::regular::X,
                    FontId::proportional(14.0),
                    theme::pal().text,
                );
                close |= resp.clicked();
            }
            inner
        });
        if open.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close && let Some(flag) = open {
            *flag = false;
        }
        out.and_then(|r| r.inner)
    }
}

/// A two-column form laid out like the Settings rows: a fixed label column
/// in faint text, controls aligned in the second.
pub(crate) fn form<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id).num_columns(2).min_col_width(110.0).spacing([14.0, 8.0]).show(ui, add).inner
}
