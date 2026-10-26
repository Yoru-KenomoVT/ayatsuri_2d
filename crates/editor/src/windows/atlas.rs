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
    /// View zoom (1 shows the whole page in the square) and where the
    /// page's corner sits from the square's, in screen points.
    zoom: f32,
    pan: Vec2,
    /// Dragging empty space or with the middle button moves the view.
    panning: bool,
}

/// The sizes the page picker offers.
const SIZES: [u32; 5] = [1024, 2048, 4096, 6144, 8192];

impl AtlasWin {
    pub fn new(s: &crate::session::Session) -> Self {
        let sizes = s.page_sizes();
        let pieces = crate::atlas::pieces(s.puppet.model(), &sizes, &s.art_boxes());
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
            zoom: 1.0,
            pan: Vec2::ZERO,
            panning: false,
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
        crate::windows::Dialog::new(tr("Texture Atlas"))
            .open(&mut open)
            .resizable(vec2(900.0, 680.0), vec2(560.0, 440.0))
            .show(ui.ctx(), |ui| {
                // controls across the top, wrapping when the window is narrow
                ui.horizontal_wrapped(|ui| {
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
                });
                // what it adds up to, and Apply, on a line of their own
                ui.horizontal(|ui| {
                    ui.weak(trf(
                        "{pieces} pieces on {pages} page(s)",
                        &[("pieces", &win.pieces.len()), ("pages", &win.layout.pages)],
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(egui::RichText::new(tr("Apply")).strong()).clicked() {
                            apply = true;
                        }
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
                let prev = win.page.checked_sub(1);
                let next = (win.page + 1 < pages).then_some(win.page + 1);
                let dragging = win.drag.is_some();
                let released = ui.input(|inp| inp.pointer.any_released());
                let label = trf("Page {n} / {total}", &[("n", &(win.page + 1)), ("total", &pages)]);
                // one joined control: arrow, page with its list, arrow; centred
                // on the window, the piece count beside it
                const ARROW: f32 = 28.0;
                const PAGE_W: f32 = 132.0;
                ui.horizontal(|ui| {
                    let whole = ARROW * 2.0 + PAGE_W + 2.0;
                    ui.add_space(((ui.available_width() - whole) / 2.0).max(0.0));
                    let stroke = ui.visuals().widgets.inactive.bg_stroke;
                    egui::Frame::new()
                        .stroke(stroke)
                        .corner_radius(4.0)
                        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let divider = |ui: &mut egui::Ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                                ui.painter().rect_filled(r, 0.0, stroke.color);
                            };
                            // (clicked, a dragged piece dropped on it)
                            let arrow =
                                |ui: &mut egui::Ui, icon: &str, on: bool, hint: &'static str| {
                                    let resp = ui
                                        .add_enabled(
                                            on,
                                            egui::Button::new(icon)
                                                .frame(false)
                                                .min_size(vec2(ARROW, 22.0)),
                                        )
                                        .on_hover_text(tr(hint));
                                    (
                                        resp.clicked(),
                                        on && dragging && resp.contains_pointer() && released,
                                    )
                                };
                            let (c, d) = arrow(
                                ui,
                                egui_phosphor::regular::CARET_LEFT,
                                prev.is_some(),
                                "Previous page (drop a piece here to move it there)",
                            );
                            if c {
                                go = prev;
                            }
                            if d {
                                move_to = prev;
                            }
                            divider(ui);
                            let page_btn = ui.add_sized(
                                [PAGE_W, 22.0],
                                egui::Button::new(format!(
                                    "{label}  {}",
                                    egui_phosphor::regular::CARET_DOWN
                                ))
                                .frame(false),
                            );
                            egui::Popup::menu(&page_btn).show(|ui| {
                                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                    for i in 0..pages {
                                        let item = trf(
                                            "Page {n} ({count})",
                                            &[("n", &(i + 1)), ("count", &count_on(i))],
                                        );
                                        if ui.selectable_label(win.page == i, item).clicked() {
                                            go = Some(i);
                                        }
                                    }
                                });
                            });
                            divider(ui);
                            let (c, d) = arrow(
                                ui,
                                egui_phosphor::regular::CARET_RIGHT,
                                next.is_some(),
                                "Next page (drop a piece here to move it there)",
                            );
                            if c {
                                go = next;
                            }
                            if d {
                                move_to = next;
                            }
                        });
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
                // zoom: the wheel or a pinch around the pointer, these buttons,
                // or Fit; drag empty space (or with the middle button) to pan
                let mut fit = false;
                let mut step_zoom: Option<f32> = None;
                ui.horizontal(|ui| {
                    ui.weak(tr("Scroll to zoom, drag empty space to pan"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr("Fit")).clicked() {
                            fit = true;
                        }
                        if ui.button(egui_phosphor::regular::PLUS).clicked() {
                            step_zoom = Some(1.25);
                        }
                        ui.label(format!("{:.0}%", win.zoom * 100.0));
                        if ui.button(egui_phosphor::regular::MINUS).clicked() {
                            step_zoom = Some(0.8);
                        }
                    });
                });
                // the page: a square, pieces drawn from their old page's thumbnail
                let side = ui.available_height().min(ui.available_width() - 220.0).max(300.0);
                ui.horizontal_top(|ui| {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, 3.0, Color32::from_gray(30));
                    let n = win.layout.page_size as f32;
                    // zoom about a screen point, keeping the page point under it
                    let zoom_at = |win: &mut AtlasWin, at: Pos2, factor: f32| {
                        let old = side * win.zoom / n;
                        let page_pt = (at - rect.min - win.pan) / old;
                        win.zoom = (win.zoom * factor).clamp(0.5, 32.0);
                        let new = side * win.zoom / n;
                        win.pan = at - rect.min - page_pt * new;
                    };
                    if fit || (resp.double_clicked() && !win.panning && win.drag.is_none()) {
                        win.zoom = 1.0;
                        win.pan = Vec2::ZERO;
                    }
                    if let Some(f) = step_zoom {
                        zoom_at(win, rect.center(), f);
                    }
                    if resp.hovered() {
                        let (scroll, pinch, at) = ui.input(|i| {
                            (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos())
                        });
                        if let Some(at) = at {
                            let factor = if pinch != 1.0 { pinch } else { (scroll / 300.0).exp() };
                            if (factor - 1.0).abs() > 1e-4 {
                                zoom_at(win, at, factor);
                            }
                        }
                    }
                    // the page stays at least partly on screen
                    let scale = side * win.zoom / n;
                    let page_px = n * scale;
                    win.pan.x = win.pan.x.clamp(-page_px + 40.0, side - 40.0);
                    win.pan.y = win.pan.y.clamp(-page_px + 40.0, side - 40.0);
                    let origin = rect.min + win.pan;
                    let to_screen =
                        |x: f32, y: f32| pos2(origin.x + x * scale, origin.y + y * scale);
                    let from_screen =
                        |p: Pos2| ((p.x - origin.x) / scale, (p.y - origin.y) / scale);
                    let page_rect = Rect::from_min_size(origin, vec2(page_px, page_px));
                    painter.rect_filled(page_rect, 0.0, Color32::from_gray(40));
                    // a faint grid every 256 px (finer when zoomed in), so sizes
                    // read at a glance
                    let grid = if scale * 64.0 > 24.0 { 64.0 } else { 256.0 };
                    let c = Color32::from_gray(52);
                    let mut g = 0.0;
                    while g <= n {
                        let x = origin.x + g * scale;
                        let y = origin.y + g * scale;
                        if x >= rect.min.x && x <= rect.max.x {
                            painter.line_segment(
                                [
                                    pos2(x, page_rect.min.y.max(rect.min.y)),
                                    pos2(x, page_rect.max.y.min(rect.max.y)),
                                ],
                                Stroke::new(1.0, c),
                            );
                        }
                        if y >= rect.min.y && y <= rect.max.y {
                            painter.line_segment(
                                [
                                    pos2(page_rect.min.x.max(rect.min.x), y),
                                    pos2(page_rect.max.x.min(rect.max.x), y),
                                ],
                                Stroke::new(1.0, c),
                            );
                        }
                        g += grid;
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
                    // panning: the middle button anywhere, or the left on empty space
                    let middle = ui.input(|i| i.pointer.middle_down());
                    if resp.drag_started() && (middle || hit.is_none()) {
                        win.panning = true;
                    }
                    if win.panning {
                        win.pan += resp.drag_delta();
                    }
                    if resp.drag_started()
                        && !win.panning
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
                        win.panning = false;
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
