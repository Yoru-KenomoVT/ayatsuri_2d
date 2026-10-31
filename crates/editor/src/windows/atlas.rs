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
    /// A page size typed in rather than picked from the list.
    custom: bool,
    /// What Verify placement last said, when it found nothing.
    verified: bool,
    /// Auto layout scales every piece by this first (Fill page finds it).
    scale: f32,
}

/// The sizes the page picker offers.
const SIZES: [u32; 6] = [1024, 2048, 4096, 6144, 8192, 16384];

impl AtlasWin {
    pub fn new(s: &crate::session::Session) -> Self {
        let sizes = s.page_sizes();
        let pieces = crate::atlas::pieces(s.puppet.model(), &sizes, &s.art_boxes());
        // the current state as the starting layout: pages at their largest size
        let page_size = pieces
            .iter()
            .filter_map(|p| sizes.get(p.page))
            .map(|&(w, h)| w.max(h))
            .max()
            .unwrap_or(4096)
            .clamp(crate::atlas::MIN_PAGE, crate::atlas::MAX_PAGE);
        // the pages meshes draw from now, in order (the old ones stay for
        // undo, and the ArtPath swatch is not part of the atlas)
        let mut used: Vec<usize> = pieces.iter().map(|p| p.page).collect();
        used.sort_unstable();
        used.dedup();
        let page_count = used.len().max(1);
        let places = pieces
            .iter()
            .map(|p| Place::at(used.iter().position(|&u| u == p.page).unwrap_or(0), p.x, p.y))
            .collect();
        let mut win = AtlasWin {
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
            custom: false,
            verified: false,
            scale: 1.0,
        };
        // the layers still on textures of their own: start from a packed
        // layout to make the atlas from, at the size most models use
        if s.atlas_pending() {
            win.layout.page_size = crate::gpu_view::max_texture_side()
                .unwrap_or(4096)
                .clamp(crate::atlas::MIN_PAGE, 4096);
            win.auto();
        }
        win
    }

    /// Each piece's turns as placed now, which a new layout keeps.
    fn turns(&self) -> Vec<u8> {
        self.layout.places.iter().map(|p| p.turns).collect()
    }

    fn auto(&mut self) {
        let turns = self.turns();
        match crate::atlas::auto_layout_scaled(
            &self.pieces,
            self.layout.page_size,
            self.fit,
            self.scale,
            &turns,
        ) {
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
                    // the GPU's own limit, or a safe guess before it has drawn
                    let gpu_max = crate::gpu_view::max_texture_side()
                        .unwrap_or(16384)
                        .clamp(crate::atlas::MIN_PAGE, crate::atlas::MAX_PAGE);
                    let custom = !SIZES.contains(&win.layout.page_size) || win.custom;
                    egui::ComboBox::from_id_salt("atlas-size")
                        .selected_text(if custom {
                            tr("Custom").to_string()
                        } else {
                            format!("{} px", win.layout.page_size)
                        })
                        .show_ui(ui, |ui| {
                            for sz in SIZES.into_iter().filter(|&sz| sz <= gpu_max) {
                                if ui
                                    .selectable_label(
                                        !custom && win.layout.page_size == sz,
                                        format!("{sz} px"),
                                    )
                                    .clicked()
                                {
                                    win.custom = false;
                                    win.layout.page_size = sz;
                                    win.auto();
                                }
                            }
                            if ui.selectable_label(custom, tr("Custom…")).clicked() {
                                win.custom = true;
                            }
                        });
                    if custom {
                        let r = ui.add(
                            egui::DragValue::new(&mut win.layout.page_size)
                                .range(crate::atlas::MIN_PAGE..=gpu_max)
                                .speed(16.0)
                                .suffix(" px"),
                        );
                        if r.drag_stopped() || r.lost_focus() {
                            win.auto();
                        }
                    }
                    let side = win.layout.page_size;
                    if side > 8192 {
                        // RGBA, plus about a third more for the mip levels
                        let mb = (side as u64 * side as u64 * 4) * 4 / 3 / (1024 * 1024);
                        ui.colored_label(
                            crate::theme::pal().rotation_red,
                            crate::i18n::trf(
                                "{mb} MB of GPU memory per page. Some apps and GPUs can't load pages over 8192 px",
                                &[("mb", &mb)],
                            ),
                        );
                    }
                    ui.checkbox(&mut win.fit, tr("Scale pieces that don't fit"));
                    // every piece scaled together; Fill page finds the most
                    // that still fits one page
                    ui.label(tr("Scale"));
                    let sc = ui.add(
                        egui::DragValue::new(&mut win.scale)
                            .range(0.01..=16.0)
                            .speed(0.01)
                            .max_decimals(2),
                    );
                    if sc.drag_stopped() || sc.lost_focus() {
                        win.auto();
                    }
                    if ui
                        .button(tr("Fill page"))
                        .on_hover_text(tr(
                            "Scale every piece up (or down) as far as they all still fit one page",
                        ))
                        .clicked()
                    {
                        let turns = win.turns();
                        win.scale =
                            crate::atlas::fill_scale(&win.pieces, win.layout.page_size, &turns, 16.0);
                        win.auto();
                    }
                    if ui.button(tr("Auto layout")).clicked() {
                        win.auto();
                    }
                    // Cubism's Verify placement: select the next piece that
                    // overlaps another or reaches past the page
                    if ui
                        .button(tr("Verify placement"))
                        .on_hover_text(tr(
                            "Select the pieces that overlap another or reach past the page, one at a time",
                        ))
                        .clicked()
                    {
                        let problems = crate::atlas::verify(&win.pieces, &win.layout);
                        let mut flagged: Vec<usize> = problems
                            .overlapping
                            .iter()
                            .chain(&problems.overhanging)
                            .copied()
                            .collect();
                        flagged.sort_unstable();
                        flagged.dedup();
                        let next = match win.selected {
                            Some(cur) => flagged.iter().find(|&&i| i > cur).or(flagged.first()),
                            None => flagged.first(),
                        };
                        win.verified = next.is_none();
                        if let Some(&i) = next {
                            win.selected = Some(i);
                            if let Some(pl) = win.layout.places.get(i) {
                                win.page = pl.page;
                            }
                        }
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
                    let problems = crate::atlas::verify(&win.pieces, &win.layout);
                    for (i, (p, pl)) in win.pieces.iter().zip(&win.layout.places).enumerate() {
                        if pl.page != win.page {
                            continue;
                        }
                        let (w, h) = pl.size(p);
                        let r = Rect::from_min_max(
                            to_screen(pl.x as f32, pl.y as f32),
                            to_screen(pl.x as f32 + w as f32, pl.y as f32 + h as f32),
                        );
                        if let (Some(Some(t)), Some(&(pw, ph))) =
                            (s.textures.get(p.page), win.sizes.get(p.page))
                        {
                            // the piece's corners, turned as it is placed
                            let uv = |x: u32, y: u32| pos2(x as f32 / pw as f32, y as f32 / ph as f32);
                            let corners = [
                                ((0.0, 0.0), uv(p.x, p.y)),
                                ((p.w as f32, 0.0), uv(p.x + p.w, p.y)),
                                ((p.w as f32, p.h as f32), uv(p.x + p.w, p.y + p.h)),
                                ((0.0, p.h as f32), uv(p.x, p.y + p.h)),
                            ];
                            let mut mesh = egui::Mesh::with_texture(t.id());
                            for ((cx, cy), tuv) in corners {
                                let (x, y) = pl.map(p, cx, cy);
                                mesh.vertices.push(egui::epaint::Vertex {
                                    pos: to_screen(x, y),
                                    uv: tuv,
                                    color: Color32::WHITE,
                                });
                            }
                            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                            painter.add(egui::Shape::mesh(mesh));
                        }
                        let on = win.selected == Some(i);
                        // a piece overlapping another, or past the page, in red
                        let bad = problems.contains(i);
                        painter.rect_stroke(
                            r,
                            0.0,
                            Stroke::new(
                                if on || bad { 2.0 } else { 1.0 },
                                if on {
                                    crate::theme::pal().accent
                                } else if bad {
                                    crate::theme::pal().rotation_red
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
                        let (w, h) = win.pieces.get(i).map_or((0, 0), |p| pl.size(p));
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
                        let found = crate::atlas::verify(&win.pieces, &win.layout);
                        if !found.is_empty() {
                            win.verified = false;
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    egui_phosphor::regular::WARNING,
                                    trf(
                                        "{overlap} overlapping, {over} past the page: they would show other pixels on the model",
                                        &[
                                            ("overlap", &found.overlapping.len()),
                                            ("over", &found.overhanging.len()),
                                        ],
                                    )
                                ))
                                .size(11.0)
                                .color(crate::theme::pal().rotation_red),
                            );
                        } else if win.verified {
                            ui.weak(tr("Every piece has its own space"));
                        }
                        egui::ScrollArea::vertical().max_height(side - 24.0).show(ui, |ui| {
                            for (i, (p, pl)) in
                                win.pieces.iter().zip(&win.layout.places).enumerate()
                            {
                                if pl.page != win.page {
                                    continue;
                                }
                                let name = s.mesh_name(p.mesh);
                                let mark = if found.contains(i) {
                                    format!("{} ", egui_phosphor::regular::WARNING)
                                } else {
                                    String::new()
                                };
                                if ui
                                    .selectable_label(
                                        win.selected == Some(i),
                                        format!("{mark}{name}  {}×{}", p.w, p.h),
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
                            // this piece's own scale and turn; copies of the
                            // same rectangle go with it
                            let key = (p.page, p.x, p.y, p.w, p.h, p.art);
                            let twins: Vec<usize> = win
                                .pieces
                                .iter()
                                .enumerate()
                                .filter(|(_, q)| (q.page, q.x, q.y, q.w, q.h, q.art) == key)
                                .map(|(j, _)| j)
                                .collect();
                            if let Some(pl) = win.layout.places.get(i).cloned() {
                                let mut next = pl.clone();
                                ui.horizontal(|ui| {
                                    ui.label(tr("Scale"));
                                    ui.add(
                                        egui::DragValue::new(&mut next.scale)
                                            .range(0.01..=16.0)
                                            .speed(0.01)
                                            .max_decimals(2),
                                    );
                                    if ui
                                        .button(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE)
                                        .on_hover_text(tr("Turn a quarter left"))
                                        .clicked()
                                    {
                                        next.turns = (next.turns + 3) % 4;
                                    }
                                    if ui
                                        .button(egui_phosphor::regular::ARROW_CLOCKWISE)
                                        .on_hover_text(tr("Turn a quarter right"))
                                        .clicked()
                                    {
                                        next.turns = (next.turns + 1) % 4;
                                    }
                                });
                                if next != pl {
                                    for j in twins {
                                        if let Some(t) = win.layout.places.get_mut(j) {
                                            t.scale = next.scale;
                                            t.turns = next.turns;
                                        }
                                    }
                                }
                            }
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
