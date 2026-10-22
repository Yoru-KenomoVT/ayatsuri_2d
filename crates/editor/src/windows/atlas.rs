// Modeling > Texture Atlas: the pages laid out as Cubism's atlas editor
// shows them, each mesh's rectangle drawn with its own pixels. Pieces drag
// within a page or onto another page's tab, Auto layout packs everything
// onto pages of the chosen size, and Apply rewrites pixels and uvs in one
// undo step. Nothing changes on the model until Apply.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::atlas::{GAP, Layout, Piece, Place};
use crate::i18n::{tr, trf};
use crate::panels::EditorUi;

pub struct AtlasWin {
    pieces: Vec<Piece>,
    layout: Layout,
    /// The old pages' sizes, for drawing pieces from their thumbnails.
    sizes: Vec<(u32, u32)>,
    page: usize,
    selected: Option<usize>,
    /// The piece being dragged and the pointer's offset from its corner.
    drag: Option<(usize, Vec2)>,
    fit: bool,
    error: Option<String>,
}

/// The sizes the page picker offers.
const SIZES: [u32; 5] = [1024, 2048, 4096, 6144, 8192];

impl AtlasWin {
    pub fn new(s: &crate::session::Session) -> Self {
        let sizes = s.page_sizes();
        let pieces = crate::atlas::pieces(s.puppet.model(), &sizes);
        // the current state as the starting layout: pages at their largest size
        let page_size =
            sizes.iter().map(|&(w, h)| w.max(h)).max().unwrap_or(4096).clamp(256, 16384);
        let swatch = s.puppet.model().art_path_page.map(|p| p as usize);
        let page_count = sizes.len().saturating_sub(usize::from(swatch.is_some())).max(1);
        let places = pieces.iter().map(|p| Place { page: p.page, x: p.x, y: p.y }).collect();
        AtlasWin {
            pieces,
            layout: Layout { page_size, pages: page_count, places },
            sizes,
            page: 0,
            selected: None,
            drag: None,
            fit: true,
            error: None,
        }
    }

    fn auto(&mut self) {
        match crate::atlas::auto_layout(&self.pieces, self.layout.page_size, self.fit) {
            Ok(l) => {
                self.layout = l;
                self.page = 0;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

impl EditorUi {
    pub(crate) fn open_atlas(&mut self) {
        let Some(s) = &self.session else { return };
        self.atlas_win = Some(AtlasWin::new(s));
    }

    pub(crate) fn atlas_window(&mut self, ui: &mut Ui) {
        let Some(win) = &mut self.atlas_win else { return };
        let Some(s) = &self.session else {
            self.atlas_win = None;
            return;
        };
        let mut open = true;
        let mut apply = false;
        egui::Window::new(tr("Texture Atlas"))
            .collapsible(false)
            .open(&mut open)
            .default_size([900.0, 640.0])
            .show(ui.ctx(), |ui| {
                // controls across the top
                ui.horizontal(|ui| {
                    ui.label(tr("Page size"));
                    egui::ComboBox::from_id_salt("atlas-size")
                        .selected_text(format!("{} px", win.layout.page_size))
                        .show_ui(ui, |ui| {
                            for sz in SIZES {
                                if ui
                                    .selectable_label(
                                        win.layout.page_size == sz,
                                        format!("{sz} px"),
                                    )
                                    .clicked()
                                {
                                    win.layout.page_size = sz;
                                    win.auto();
                                }
                            }
                        });
                    ui.checkbox(&mut win.fit, tr("Scale pieces that don't fit"));
                    if ui.button(tr("Auto layout")).clicked() {
                        win.auto();
                    }
                    ui.separator();
                    if ui.button(tr("New page")).clicked() {
                        win.layout.pages += 1;
                        win.page = win.layout.pages - 1;
                    }
                    let empty = !win.layout.places.iter().any(|p| p.page == win.page);
                    if ui
                        .add_enabled(
                            empty && win.layout.pages > 1,
                            egui::Button::new(tr("Delete page")),
                        )
                        .clicked()
                    {
                        let gone = win.page;
                        win.layout.pages -= 1;
                        for p in &mut win.layout.places {
                            if p.page > gone {
                                p.page -= 1;
                            }
                        }
                        win.page = win.page.min(win.layout.pages - 1);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(egui::RichText::new(tr("Apply")).strong()).clicked() {
                            apply = true;
                        }
                        ui.weak(trf(
                            "{pieces} pieces on {pages} page(s)",
                            &[("pieces", &win.pieces.len()), ("pages", &win.layout.pages)],
                        ));
                    });
                });
                if let Some(e) = &win.error {
                    ui.colored_label(crate::theme::pal().accent, e);
                }
                // the page stepper: arrows, the page number with a list to jump
                // from, and a dragged piece dropped on an arrow moves to that page
                let pages = win.layout.pages.max(1);
                let count_on = |i: usize| win.layout.places.iter().filter(|p| p.page == i).count();
                let mut go: Option<usize> = None;
                let mut move_to: Option<usize> = None;
                ui.horizontal(|ui| {
                    let dragging = win.drag.is_some();
                    let released = ui.input(|inp| inp.pointer.any_released());
                    // (clicked, a dragged piece dropped on it)
                    let arrow = |ui: &mut egui::Ui, icon: &str, on: bool, hint: &'static str| {
                        let resp = ui
                            .add_enabled(on, egui::Button::new(icon).min_size(vec2(28.0, 22.0)))
                            .on_hover_text(tr(hint));
                        (resp.clicked(), on && dragging && resp.contains_pointer() && released)
                    };
                    let prev = win.page.checked_sub(1);
                    let next = (win.page + 1 < pages).then_some(win.page + 1);
                    let (clicked, dropped) = arrow(
                        ui,
                        egui_phosphor::regular::CARET_LEFT,
                        prev.is_some(),
                        "Previous page (drop a piece here to move it there)",
                    );
                    if clicked {
                        go = prev;
                    }
                    if dropped {
                        move_to = prev;
                    }
                    egui::ComboBox::from_id_salt("atlas-page")
                        .selected_text(trf(
                            "Page {n} / {total}",
                            &[("n", &(win.page + 1)), ("total", &pages)],
                        ))
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for i in 0..pages {
                                let label = trf(
                                    "Page {n} ({count})",
                                    &[("n", &(i + 1)), ("count", &count_on(i))],
                                );
                                if ui.selectable_label(win.page == i, label).clicked() {
                                    go = Some(i);
                                }
                            }
                        });
                    let (clicked, dropped) = arrow(
                        ui,
                        egui_phosphor::regular::CARET_RIGHT,
                        next.is_some(),
                        "Next page (drop a piece here to move it there)",
                    );
                    if clicked {
                        go = next;
                    }
                    if dropped {
                        move_to = next;
                    }
                    ui.weak(trf("{count} pieces", &[("count", &count_on(win.page))]));
                });
                if let Some(t) = move_to
                    && let Some((pi, _)) = win.drag
                    && let Some(pl) = win.layout.places.get_mut(pi)
                {
                    pl.page = t;
                    pl.x = GAP;
                    pl.y = GAP;
                    win.page = t;
                } else if let Some(t) = go {
                    win.page = t;
                }
                ui.separator();
                // the page: a square, pieces drawn from their old page's thumbnail
                let side = ui.available_height().min(ui.available_width() - 220.0).max(300.0);
                ui.horizontal_top(|ui| {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 3.0, Color32::from_gray(40));
                    let n = win.layout.page_size as f32;
                    let to_screen =
                        |x: f32, y: f32| pos2(rect.min.x + x / n * side, rect.min.y + y / n * side);
                    let from_screen =
                        |p: Pos2| ((p.x - rect.min.x) / side * n, (p.y - rect.min.y) / side * n);
                    // a faint grid every 256 px, so sizes read at a glance
                    let step = 256.0 / n * side;
                    let mut g = 0.0;
                    while g < side {
                        let c = Color32::from_gray(52);
                        painter.line_segment(
                            [pos2(rect.min.x + g, rect.min.y), pos2(rect.min.x + g, rect.max.y)],
                            Stroke::new(1.0, c),
                        );
                        painter.line_segment(
                            [pos2(rect.min.x, rect.min.y + g), pos2(rect.max.x, rect.min.y + g)],
                            Stroke::new(1.0, c),
                        );
                        g += step;
                    }
                    let pointer = resp.interact_pointer_pos();
                    let mut hit: Option<usize> = None;
                    for (i, (p, pl)) in win.pieces.iter().zip(&win.layout.places).enumerate() {
                        if pl.page != win.page {
                            continue;
                        }
                        let (w, h) = (
                            p.w.min(win.layout.page_size) as f32,
                            p.h.min(win.layout.page_size) as f32,
                        );
                        let r = Rect::from_min_max(
                            to_screen(pl.x as f32, pl.y as f32),
                            to_screen(pl.x as f32 + w, pl.y as f32 + h),
                        );
                        if let (Some(Some(t)), Some(&(pw, ph))) =
                            (s.textures.get(p.page), win.sizes.get(p.page))
                        {
                            let uv = Rect::from_min_max(
                                pos2(p.x as f32 / pw as f32, p.y as f32 / ph as f32),
                                pos2(
                                    (p.x + p.w) as f32 / pw as f32,
                                    (p.y + p.h) as f32 / ph as f32,
                                ),
                            );
                            painter.image(t.id(), r, uv, Color32::WHITE);
                        }
                        let on = win.selected == Some(i);
                        painter.rect_stroke(
                            r,
                            0.0,
                            Stroke::new(
                                if on { 2.0 } else { 1.0 },
                                if on {
                                    crate::theme::pal().accent
                                } else {
                                    Color32::from_white_alpha(90)
                                },
                            ),
                            egui::StrokeKind::Inside,
                        );
                        if pointer.is_some_and(|pt| r.contains(pt)) {
                            hit = Some(i);
                        }
                    }
                    if resp.drag_started()
                        && let Some(i) = hit
                        && let Some(pt) = pointer
                        && let Some(pl) = win.layout.places.get(i)
                    {
                        let corner = to_screen(pl.x as f32, pl.y as f32);
                        win.drag = Some((i, pt - corner));
                        win.selected = Some(i);
                    } else if resp.clicked() {
                        win.selected = hit;
                    }
                    if let Some((i, off)) = win.drag
                        && let Some(pt) = pointer
                        && resp.dragged()
                        && let Some(pl) = win.layout.places.get_mut(i)
                    {
                        let (x, y) = from_screen(pt - off);
                        let (w, h) = win.pieces.get(i).map_or((0, 0), |p| (p.w, p.h));
                        pl.x =
                            (x.round().max(0.0) as u32).min(win.layout.page_size.saturating_sub(w));
                        pl.y =
                            (y.round().max(0.0) as u32).min(win.layout.page_size.saturating_sub(h));
                    }
                    if ui.input(|inp| inp.pointer.any_released()) {
                        win.drag = None;
                    }
                    // the piece list for this page
                    ui.vertical(|ui| {
                        ui.set_min_width(200.0);
                        ui.strong(tr("Pieces on this page"));
                        egui::ScrollArea::vertical().max_height(side - 24.0).show(ui, |ui| {
                            for (i, (p, pl)) in
                                win.pieces.iter().zip(&win.layout.places).enumerate()
                            {
                                if pl.page != win.page {
                                    continue;
                                }
                                let name = s.mesh_name(p.mesh);
                                if ui
                                    .selectable_label(
                                        win.selected == Some(i),
                                        format!("{name}  {}×{}", p.w, p.h),
                                    )
                                    .clicked()
                                {
                                    win.selected = Some(i);
                                }
                            }
                        });
                        if let Some(i) = win.selected
                            && let Some(p) = win.pieces.get(i)
                        {
                            ui.separator();
                            ui.weak(trf(
                                "{name}: {w} × {h} px, from page {page}",
                                &[
                                    ("name", &s.mesh_name(p.mesh)),
                                    ("w", &p.w),
                                    ("h", &p.h),
                                    ("page", &p.page),
                                ],
                            ));
                        }
                    });
                });
            });
        if apply {
            let (pieces, layout) = (win.pieces.clone(), win.layout.clone());
            let ctx = ui.ctx().clone();
            let Some(s) = &mut self.session else { return };
            match s.apply_atlas(&ctx, &pieces, &layout) {
                Ok(()) => self.atlas_win = None,
                Err(e) => {
                    log::error!("atlas: {e}");
                    if let Some(w) = &mut self.atlas_win {
                        w.error = Some(e);
                    }
                }
            }
        } else if !open {
            self.atlas_win = None;
        }
    }
}
