// The one shell every floating dialog uses, so they all match Settings and
// About: frameless, no outline, a title strip that moves the window, the
// same close button, the same padding. Positions live in egui's memory per
// title, so a dialog reopens where it was left.

use egui::{Align2, FontId, Id, Margin, Pos2, Rect, Sense, Ui, vec2};

use crate::theme;

const HEADER: f32 = 40.0;

pub(crate) struct Dialog<'a> {
    title: String,
    open: Option<&'a mut bool>,
    width: f32,
    at: Option<Pos2>,
}

impl<'a> Dialog<'a> {
    pub(crate) fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), open: None, width: 340.0, at: None }
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

    /// Where the dialog's centre starts, instead of the window centre.
    pub(crate) fn at(mut self, centre: Pos2) -> Self {
        self.at = Some(centre);
        self
    }

    pub(crate) fn show<R>(self, ctx: &egui::Context, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let Dialog { title, open, width, at } = self;
        if open.as_deref() == Some(&false) {
            return None;
        }
        let id = Id::new(("dialog", title.as_str()));
        let pos_id = id.with("pos");
        let start = at.unwrap_or(ctx.content_rect().center());
        let pos = ctx.data(|d| d.get_temp::<Pos2>(pos_id)).unwrap_or(start);
        let style = ctx.style_of(ctx.theme());
        let mut close = false;
        let out = egui::Window::new(title.as_str())
            .id(id)
            .title_bar(false)
            // only the header strip moves it; egui's own drag-anywhere
            // fought the pinned position and snapped back
            .movable(false)
            .collapsible(false)
            .resizable(false)
            .frame(egui::Frame::window(&style).inner_margin(0.0).stroke(egui::Stroke::NONE))
            .pivot(Align2::CENTER_CENTER)
            .current_pos(pos)
            .show(ctx, |ui| {
                ui.set_width(width);
                let (strip, _) = ui.allocate_exact_size(vec2(width, HEADER), Sense::hover());
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
                egui::Frame::NONE
                    .inner_margin(Margin { left: 18, right: 18, top: 4, bottom: 16 })
                    .show(ui, |ui| {
                        ui.set_width(width - 36.0);
                        add(ui)
                    })
                    .inner
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
