//! Motion playback: the model's scenes and compositions, started by index,
//! faded in over the pose and out again when the next one starts. The
//! player writes parameters during `ay2d_update`, before physics, so a
//! host that also sets parameters by hand sees the motion win on the
//! curves it has and its own values everywhere else. Events a motion
//! carries are queued for the host to poll; nothing calls back.

use std::collections::HashMap;
use std::ffi::{CString, c_char};

use ayatsuri_core::Puppet;
use ayatsuri_core::comp::{self, Scope};
use ayatsuri_format::{Animation, Composition, CurveTarget};

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, cstr_in, model, model_mut, output, put};

/// The spec's fade when a motion names none.
const DEFAULT_FADE: f32 = 1.0;

/// What a curve drives, resolved once at load.
#[derive(Clone, Copy)]
pub(crate) enum Drive {
    Param(usize),
    Part(usize),
}

/// One thing the player can run: a scene or a composition of the file.
pub(crate) struct Clip {
    pub id: CString,
    pub source: Source,
    pub duration: f32,
    pub fps: f32,
    pub looped: bool,
    pub fade_in: Option<f32>,
    pub fade_out: Option<f32>,
    /// Time and NUL-terminated text of each event.
    pub events: Vec<(f32, CString)>,
}

pub(crate) enum Source {
    /// A scene, with each of its curves resolved to what it drives
    /// (None for curves on things the model does not have).
    Scene(usize, Vec<Option<Drive>>),
    Comp(usize),
}

/// A clip on the player: where it is, and how far faded in or out.
struct Playing {
    clip: usize,
    time: f32,
    speed: f32,
    looping: bool,
    fade_in: f32,
    elapsed: f32,
    /// Set when the next clip took over: the weight it had then, how long
    /// it takes to go, and how far along that is.
    out: Option<(f32, f32, f32)>,
    finished: bool,
}

impl Playing {
    fn weight(&self) -> f32 {
        let w_in = if self.fade_in <= 0.0 { 1.0 } else { (self.elapsed / self.fade_in).min(1.0) };
        match self.out {
            Some((from, dur, gone)) => {
                if dur <= 0.0 {
                    0.0
                } else {
                    from * (1.0 - gone / dur).max(0.0)
                }
            }
            None => w_in,
        }
    }
}

/// Facts about a clip, for a host's motion list.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MotionInfo {
    /// Seconds.
    pub duration: f32,
    pub fps: f32,
    /// 1 when the clip was authored to loop.
    pub looped: i32,
    /// 1 when the clip is a composition rather than a scene.
    pub composition: i32,
    /// Authored fades in seconds; -1 when the clip leaves them to the player.
    pub fade_in: f32,
    pub fade_out: f32,
    pub event_count: u32,
}

/// An event a motion fired since the last update.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Event {
    /// The motion that fired it, by index.
    pub motion: u32,
    /// Its time on the motion's clock, in seconds.
    pub time: f32,
    /// The event's text, NUL-terminated, alive as long as the model.
    pub name: *const c_char,
}

#[derive(Default)]
pub(crate) struct Player {
    pub clips: Vec<Clip>,
    playing: Vec<Playing>,
    fired: Vec<Event>,
}

fn read(puppet: &Puppet, drive: Drive) -> f32 {
    match drive {
        Drive::Param(i) => puppet.value(i),
        Drive::Part(i) => puppet.part_user_opacity(i),
    }
}

/// Mix `v` over what is there by weight `w`.
fn write(puppet: &mut Puppet, drive: Drive, v: f32, w: f32) {
    let base = read(puppet, drive);
    let mixed = base + (v - base) * w;
    match drive {
        Drive::Param(i) => puppet.set_value(i, mixed),
        Drive::Part(i) => puppet.set_part_opacity(i, mixed),
    }
}

pub(crate) fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Player {
    pub(crate) fn new(
        scenes: &[Animation],
        comps: &[Composition],
        params: &HashMap<String, usize>,
        parts: &HashMap<String, usize>,
    ) -> Self {
        let mut clips = Vec::with_capacity(scenes.len() + comps.len());
        for (i, a) in scenes.iter().enumerate() {
            let drives = a
                .curves
                .iter()
                .map(|c| match c.target {
                    CurveTarget::Parameter => params.get(&c.id).map(|&p| Drive::Param(p)),
                    CurveTarget::PartOpacity => parts.get(&c.id).map(|&p| Drive::Part(p)),
                    CurveTarget::Model => None,
                })
                .collect();
            clips.push(Clip {
                id: cstring(&a.name),
                source: Source::Scene(i, drives),
                duration: a.duration.max(0.0),
                fps: a.fps,
                looped: a.looped,
                fade_in: a.fade_in,
                fade_out: a.fade_out,
                events: a.events.iter().map(|e| (e.time, cstring(&e.value))).collect(),
            });
        }
        for (i, c) in comps.iter().enumerate() {
            clips.push(Clip {
                id: cstring(&c.name),
                source: Source::Comp(i),
                duration: c.duration.max(0.0),
                fps: c.fps,
                looped: c.looped,
                fade_in: None,
                fade_out: None,
                events: Vec::new(),
            });
        }
        Self { clips, playing: Vec::new(), fired: Vec::new() }
    }

    fn play(&mut self, clip: usize, looping: bool, fade_in: f32) {
        let authored = self.clips.get(clip).and_then(|c| c.fade_in);
        let fade_in = if fade_in >= 0.0 { fade_in } else { authored.unwrap_or(DEFAULT_FADE) };
        self.stop(-1.0);
        self.playing.push(Playing {
            clip,
            time: 0.0,
            speed: 1.0,
            looping,
            fade_in,
            elapsed: 0.0,
            out: None,
            finished: false,
        });
    }

    fn stop(&mut self, fade_out: f32) {
        for p in &mut self.playing {
            if p.out.is_some() {
                continue;
            }
            let authored = self.clips.get(p.clip).and_then(|c| c.fade_out);
            let dur = if fade_out >= 0.0 { fade_out } else { authored.unwrap_or(DEFAULT_FADE) };
            p.out = Some((p.weight(), dur, 0.0));
        }
    }

    /// The clip in front: the newest one not on its way out.
    fn current(&self) -> Option<&Playing> {
        self.playing.iter().rev().find(|p| p.out.is_none())
    }

    fn current_mut(&mut self) -> Option<&mut Playing> {
        self.playing.iter_mut().rev().find(|p| p.out.is_none())
    }

    /// Move every clip on by `dt`, drop the ones faded out, queue events.
    pub(crate) fn advance(&mut self, dt: f32) {
        self.fired.clear();
        for p in &mut self.playing {
            p.elapsed += dt;
            if let Some((_, _, gone)) = &mut p.out {
                *gone += dt;
            }
            let Some(c) = self.clips.get(p.clip) else { continue };
            if p.finished {
                continue;
            }
            let before = p.time;
            let mut t = p.time + dt * p.speed;
            if c.duration <= 0.0 {
                t = 0.0;
            } else if t >= c.duration {
                if p.looping || c.looped {
                    t %= c.duration;
                } else {
                    t = c.duration;
                    p.finished = true;
                }
            }
            // events crossed, with the wrap of a loop
            let crossed = |at: f32| {
                if t >= before { at > before && at <= t } else { at > before || at <= t }
            };
            for (at, name) in &c.events {
                if dt > 0.0 && crossed(*at) {
                    self.fired.push(Event {
                        motion: p.clip as u32,
                        time: *at,
                        name: name.as_ptr(),
                    });
                }
            }
            p.time = t;
        }
        self.playing.retain(|p| p.out.is_none_or(|(_, dur, gone)| dur > 0.0 && gone < dur));
    }

    /// Write every playing clip over the pose, oldest first, each by its
    /// weight, so a crossfade lands between the two.
    pub(crate) fn apply(
        &self,
        puppet: &mut Puppet,
        scenes: &[Animation],
        comps: &[Composition],
        params: &HashMap<String, usize>,
        parts: &HashMap<String, usize>,
    ) {
        let scope = Scope { scenes, comps };
        for p in &self.playing {
            let w = p.weight();
            if w <= 0.0 {
                continue;
            }
            let Some(c) = self.clips.get(p.clip) else { continue };
            match &c.source {
                Source::Scene(si, drives) => {
                    let Some(a) = scenes.get(*si) else { continue };
                    for (curve, drive) in a.curves.iter().zip(drives) {
                        let (Some(d), Some(v)) =
                            (drive, ayatsuri_core::anim::sample(a, curve, p.time))
                        else {
                            continue;
                        };
                        write(puppet, *d, v, w);
                    }
                }
                Source::Comp(ci) => {
                    let Some(comp) = comps.get(*ci) else { continue };
                    for ((target, id), blend) in comp::sample(&scope, comp, p.time) {
                        let drive = match target {
                            CurveTarget::Parameter => params.get(&id).map(|&i| Drive::Param(i)),
                            CurveTarget::PartOpacity => parts.get(&id).map(|&i| Drive::Part(i)),
                            CurveTarget::Model => None,
                        };
                        let Some(d) = drive else { continue };
                        let base = read(puppet, d);
                        write(puppet, d, blend.over(base), w);
                    }
                }
            }
        }
    }
}

/// How many motions the model carries: its scenes, then its compositions.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.player.clips.len() as u32))
}

/// A motion's name, NUL-terminated, alive as long as the model. Null past
/// the count.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        let Ok(model) = model(m) else { return std::ptr::null() };
        match model.player.clips.get(i as usize) {
            Some(c) => c.id.as_ptr(),
            None => {
                set_error(format!("motion {i} is past the model's {}", model.player.clips.len()));
                std::ptr::null()
            }
        }
    })
}

/// The index of the motion with this name, or -1.
///
/// `m` is a live handle; `name` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_index(m: *const Model, name: *const c_char) -> i32 {
    guard(-1, || {
        let Ok(model) = model(m) else { return -1 };
        let Ok(want) = cstr_in(name) else { return -1 };
        model.player.clips.iter().position(|c| c.id.as_c_str() == want).map_or(-1, |i| i as i32)
    })
}

/// Facts about motion `i`.
///
/// `m` is a live handle; `out` points at one writable info.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_info(m: *const Model, i: u32, out: *mut MotionInfo) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let Some(c) = model.player.clips.get(i as usize) else {
            set_error(format!("motion {i} is past the model's {}", model.player.clips.len()));
            return Status::Range;
        };
        put(
            out,
            MotionInfo {
                duration: c.duration,
                fps: c.fps,
                looped: i32::from(c.looped),
                composition: i32::from(matches!(c.source, Source::Comp(_))),
                fade_in: c.fade_in.unwrap_or(-1.0),
                fade_out: c.fade_out.unwrap_or(-1.0),
                event_count: c.events.len() as u32,
            },
        )
    })
}

/// Start motion `i` from its beginning. Whatever was playing fades out
/// over its own fade (or the spec's second) while this one fades in over
/// `fade_in` seconds; `fade_in` below zero takes the motion's authored
/// fade, zero starts it at full weight. `looping` repeats it; zero plays
/// it once and holds its last frame.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_play(m: *mut Model, i: u32, looping: i32, fade_in: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if i as usize >= model.player.clips.len() {
            set_error(format!("motion {i} is past the model's {}", model.player.clips.len()));
            return Status::Range;
        }
        if !fade_in.is_finite() {
            set_error("fade is not finite");
            return Status::Range;
        }
        model.player.play(i as usize, looping != 0, fade_in);
        Status::Ok
    })
}

/// Fade every playing motion out over `fade_out` seconds (below zero: the
/// motion's own fade or the spec's second; zero: at once).
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_stop(m: *mut Model, fade_out: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !fade_out.is_finite() {
            set_error("fade is not finite");
            return Status::Range;
        }
        model.player.stop(fade_out);
        Status::Ok
    })
}

/// The motion in front (the last one started and not stopped), or -1.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_playing(m: *const Model) -> i32 {
    guard(-1, || model(m).map_or(-1, |m| m.player.current().map_or(-1, |p| p.clip as i32)))
}

/// 1 when the motion in front played once through and now holds its last
/// frame; 0 while it runs, loops, or when nothing plays.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_finished(m: *const Model) -> i32 {
    guard(0, || model(m).map_or(0, |m| m.player.current().map_or(0, |p| i32::from(p.finished))))
}

/// Where the motion in front is on its clock, in seconds; -1 when nothing
/// plays.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_time(m: *const Model) -> f32 {
    guard(-1.0, || model(m).map_or(-1.0, |m| m.player.current().map_or(-1.0, |p| p.time)))
}

/// Put the motion in front at `t` seconds on its clock.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_seek(m: *mut Model, t: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !t.is_finite() || t < 0.0 {
            set_error(format!("{t} is not a time on the clock"));
            return Status::Range;
        }
        let durations: Vec<f32> = model.player.clips.iter().map(|c| c.duration).collect();
        let Some(p) = model.player.current_mut() else {
            set_error("nothing is playing");
            return Status::Range;
        };
        let duration = durations.get(p.clip).copied().unwrap_or(0.0);
        p.time = t.min(duration);
        p.finished = false;
        Status::Ok
    })
}

/// Play the motion in front at `speed` times its rate (1 is as authored).
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_set_speed(m: *mut Model, speed: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !speed.is_finite() || speed < 0.0 {
            set_error(format!("{speed} is not a speed"));
            return Status::Range;
        }
        match model.player.current_mut() {
            Some(p) => {
                p.speed = speed;
                Status::Ok
            }
            None => {
                set_error("nothing is playing");
                Status::Range
            }
        }
    })
}

/// The events motions fired during the last `ay2d_update`, into `out`.
/// Returns how many there were; with `n` smaller only the first `n` are
/// written. Poll after each update; the list is replaced by the next one.
///
/// `m` is a live handle; `out` holds `n` writable events, or `n` is zero.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_motion_events(m: *const Model, out: *mut Event, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Ok(out) = output(out, n) else { return 0 };
        for (slot, e) in out.iter_mut().zip(&model.player.fired) {
            *slot = *e;
        }
        model.player.fired.len() as u32
    })
}
