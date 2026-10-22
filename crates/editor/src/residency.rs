// Which texture pages are on the GPU at full size. Every page has a small
// proxy on the GPU for as long as the model is open; the full page is
// uploaded when a visible mesh draws it at a zoom the proxy can't carry,
// and let go again when nothing has used it for a while, when the window
// is idle or minimised, or when the budget is full. Pages are decoded from
// their PNGs on a worker thread, one at a time, so the frame never waits
// and the decoded copy exists only on its way to the GPU.
//
// The point is the two rules in docs/goals.md: memory must not grow with
// time, and running out of video memory must never crash. A model with
// more texture than the GPU can hold still opens; it just draws the pages
// that don't fit from their proxies.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use ayatsuri_render::TexData;

/// How long a full page stays after the last frame that drew it.
const LINGER: Duration = Duration::from_secs(5);
/// Idle (unfocused or minimised) this long and every full page goes.
const IDLE: Duration = Duration::from_secs(10);
/// A texture with mips costs a third more than its pixels.
const MIP_FACTOR: usize = 4;

struct Page {
    width: u32,
    height: u32,
    resident: bool,
    last_used: Instant,
}

impl Page {
    fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * MIP_FACTOR
    }
}

/// What a tick decided: a page to upload now, pages to drop.
#[derive(Default)]
pub struct Plan {
    pub upload: Option<(usize, TexData)>,
    pub drop: Vec<usize>,
}

pub struct Residency {
    pages: Vec<Page>,
    /// Full-page texture memory allowed, in bytes.
    pub budget: usize,
    /// The page being decoded, if one is.
    loading: Option<(usize, Receiver<Option<TexData>>)>,
    idle_since: Option<Instant>,
    /// How many times the GPU ran out of memory on this model.
    pub oom_count: u32,
}

impl Residency {
    pub fn new(sizes: &[(u32, u32)], budget_mb: u32) -> Self {
        let now = Instant::now();
        Residency {
            pages: sizes
                .iter()
                .map(|&(w, h)| Page { width: w, height: h, resident: false, last_used: now })
                .collect(),
            budget: budget_mb.max(64) as usize * (1 << 20),
            loading: None,
            idle_since: None,
            oom_count: 0,
        }
    }

    /// A page added or replaced after load (the ArtPath swatch, a
    /// re-import); it arrives on the GPU already, so it counts as resident.
    pub fn set_page(&mut self, index: usize, width: u32, height: u32) {
        let page = Page { width, height, resident: true, last_used: Instant::now() };
        match self.pages.get_mut(index) {
            Some(p) => *p = page,
            None => self.pages.push(page),
        }
    }

    pub fn page_size(&self, index: usize) -> Option<(u32, u32)> {
        self.pages.get(index).map(|p| (p.width, p.height))
    }

    pub fn resident_bytes(&self) -> usize {
        self.pages.iter().filter(|p| p.resident).map(Page::bytes).sum()
    }

    /// How long until an idle window's full pages are due to go, while
    /// any are still held: the editor wakes once then, as nothing else
    /// draws a frame while it sits idle.
    pub fn idle_wake(&self) -> Option<Duration> {
        let since = self.idle_since?;
        if !self.pages.iter().any(|p| p.resident) {
            return None;
        }
        Some(IDLE.saturating_sub(since.elapsed()) + Duration::from_millis(250))
    }

    pub fn resident_count(&self) -> usize {
        self.pages.iter().filter(|p| p.resident).count()
    }

    #[cfg(test)]
    pub fn is_resident(&self, index: usize) -> bool {
        self.pages.get(index).is_some_and(|p| p.resident)
    }

    /// The GPU reported out of memory: everything full goes, and the budget
    /// shrinks to what was in use so it does not happen again.
    pub fn on_out_of_memory(&mut self) -> Vec<usize> {
        self.oom_count += 1;
        let used = self.resident_bytes();
        self.budget = (used / 2).max(64 << 20);
        self.loading = None;
        let mut gone = Vec::new();
        for (i, p) in self.pages.iter_mut().enumerate() {
            if p.resident {
                p.resident = false;
                gone.push(i);
            }
        }
        gone
    }

    /// Once a frame. `used` is each page a visible mesh drew this frame and
    /// whether the proxy was too coarse for it at the current zoom; `pngs`
    /// are the encoded pages to decode from.
    pub fn tick(&mut self, used: &[(usize, bool)], idle: bool, pngs: &[Vec<u8>]) -> Plan {
        let now = Instant::now();
        let mut plan = Plan::default();

        // a decode that finished since last frame goes up now
        if let Some((i, rx)) = &self.loading {
            match rx.try_recv() {
                Ok(Some(data)) => {
                    let i = *i;
                    if let Some(p) = self.pages.get_mut(i) {
                        p.resident = true;
                        p.last_used = now;
                    }
                    plan.upload = Some((i, data));
                    self.loading = None;
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        for &(i, _) in used {
            if let Some(p) = self.pages.get_mut(i) {
                p.last_used = now;
            }
        }

        // idle long enough and the full pages are not worth their memory
        self.idle_since = if idle { self.idle_since.or(Some(now)) } else { None };
        let drop_all = self.idle_since.is_some_and(|t| now.duration_since(t) > IDLE);
        for (i, p) in self.pages.iter_mut().enumerate() {
            if p.resident && (drop_all || now.duration_since(p.last_used) > LINGER) {
                p.resident = false;
                plan.drop.push(i);
            }
        }
        if idle || self.loading.is_some() {
            return plan;
        }

        // the next page worth having at full size, most recently drawn first
        let Some(&(want, _)) = used
            .iter()
            .filter(|&&(i, need)| need && self.pages.get(i).is_some_and(|p| !p.resident))
            .max_by_key(|&&(i, _)| self.pages.get(i).map(|p| p.last_used))
        else {
            return plan;
        };
        let Some(need) = self.pages.get(want).map(Page::bytes) else { return plan };
        let Some(png) = pngs.get(want).filter(|p| !p.is_empty()).cloned() else { return plan };
        // make room: the pages nobody drew this frame go first, oldest first
        let used_now: std::collections::HashSet<usize> = used.iter().map(|&(i, _)| i).collect();
        while self.resident_bytes() + need > self.budget {
            let victim = self
                .pages
                .iter()
                .enumerate()
                .filter(|(i, p)| p.resident && !used_now.contains(i))
                .min_by_key(|(_, p)| p.last_used)
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    if let Some(p) = self.pages.get_mut(i) {
                        p.resident = false;
                    }
                    plan.drop.push(i);
                }
                // everything resident is on screen: this page stays a proxy
                None => return plan,
            }
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("texture-decode".into()).spawn(move || {
            let data = image::load_from_memory(&png).ok().map(|img| {
                let img = img.into_rgba8();
                TexData { width: img.width(), height: img.height(), rgba: img.into_raw() }
            });
            let _ = tx.send(data);
        });
        if spawned.is_ok() {
            self.loading = Some((want, rx));
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// Tick until the decode on the worker thread has landed, gathering
    /// every drop decided along the way.
    fn settle(r: &mut Residency, used: &[(usize, bool)], pngs: &[Vec<u8>]) -> Plan {
        let mut out = Plan::default();
        for _ in 0..200 {
            let plan = r.tick(used, false, pngs);
            out.drop.extend(plan.drop);
            if plan.upload.is_some() {
                out.upload = plan.upload;
                return out;
            }
            if r.loading.is_none() {
                return out;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    #[test]
    fn a_wanted_page_is_decoded_and_uploaded_once() {
        let pngs = vec![tiny_png(), tiny_png()];
        let mut r = Residency::new(&[(8, 8), (8, 8)], 64);
        let plan = settle(&mut r, &[(0, true)], &pngs);
        assert_eq!(plan.upload.as_ref().map(|(i, d)| (*i, d.width)), Some((0, 8)));
        assert!(r.is_resident(0) && !r.is_resident(1));
        let again = r.tick(&[(0, true)], false, &pngs);
        assert!(again.upload.is_none() && again.drop.is_empty());
    }

    #[test]
    fn a_proxy_that_is_fine_at_this_zoom_is_left_alone() {
        let pngs = vec![tiny_png()];
        let mut r = Residency::new(&[(8, 8)], 64);
        let plan = settle(&mut r, &[(0, false)], &pngs);
        assert!(plan.upload.is_none() && !r.is_resident(0));
    }

    #[test]
    fn the_budget_evicts_the_oldest_page_nobody_is_drawing() {
        let pngs = vec![tiny_png(), tiny_png()];
        // 8x8x4 = 256 bytes a page; the budget floor is 64 MB, so shrink it by hand
        let mut r = Residency::new(&[(8, 8), (8, 8)], 64);
        r.budget = 300;
        settle(&mut r, &[(0, true)], &pngs);
        assert!(r.is_resident(0));
        let plan = settle(&mut r, &[(1, true)], &pngs);
        assert_eq!(plan.drop, vec![0]);
        assert!(r.is_resident(1) && !r.is_resident(0));
    }

    #[test]
    fn an_idle_window_asks_to_be_woken_while_it_holds_full_pages() {
        let pngs = vec![tiny_png()];
        let mut r = Residency::new(&[(1, 1)], 512);
        settle(&mut r, &[(0, true)], &pngs);
        assert!(r.is_resident(0));
        assert_eq!(r.idle_wake(), None, "not idle yet");
        r.tick(&[], true, &pngs);
        let wait = r.idle_wake().expect("idle with a page held");
        assert!(wait <= IDLE + Duration::from_millis(250));
        // once the pages are gone there is nothing to wake for
        r.pages[0].resident = false;
        assert_eq!(r.idle_wake(), None);
    }

    #[test]
    fn out_of_memory_drops_everything_and_halves_the_budget() {
        let pngs = vec![tiny_png()];
        let mut r = Residency::new(&[(8, 8)], 64);
        settle(&mut r, &[(0, true)], &pngs);
        let gone = r.on_out_of_memory();
        assert_eq!(gone, vec![0]);
        assert!(!r.is_resident(0));
        assert_eq!(r.budget, 64 << 20);
        assert_eq!(r.oom_count, 1);
    }
}
