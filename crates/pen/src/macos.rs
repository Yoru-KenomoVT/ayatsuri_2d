// macOS: AppKit puts a tablet's pressure on its mouse events (subtype
// tablet point) and on tablet point events. A local event monitor sees them
// all before the window does, notes the pressure and hands each event on
// unchanged. Adding the monitor is the only unsafe call in the crate.
#![allow(unsafe_code)]

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventSubtype, NSEventType};

use crate::{Status, set_status, store_down, store_pressure};

pub(crate) fn start() {
    if MainThreadMarker::new().is_none() {
        log::warn!("pen tablet: the event monitor must be added on the main thread");
        return;
    }
    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseDragged
        | NSEventMask::LeftMouseUp
        | NSEventMask::TabletPoint;
    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands the monitor a live event for the length of
        // the call, and it is only read here.
        let e = unsafe { event.as_ref() };
        let kind = e.r#type();
        // subtype is only asked of mouse and tablet events, which the mask
        // limits this to
        let tablet = kind == NSEventType::TabletPoint || e.subtype() == NSEventSubtype::TabletPoint;
        if tablet {
            store_pressure(e.pressure());
            store_down(kind != NSEventType::LeftMouseUp);
        } else if kind == NSEventType::LeftMouseUp {
            store_down(false);
        }
        event.as_ptr()
    });
    // SAFETY: the block returns the event it was given, which is a valid
    // pointer, as the method requires.
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    if monitor.is_some() {
        // the monitor lives as long as the app
        std::mem::forget(monitor);
        set_status(Status::Reading(vec!["macOS tablet events".into()]));
    } else {
        log::warn!("pen tablet: AppKit refused the event monitor");
        set_status(Status::NoTablet);
    }
}
