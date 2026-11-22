// meeting_detect/panel_window.rs
//
// The floating meeting prompt's window (macOS): a borderless non-activating panel that shows over
// whatever app is in front, on every Space and over a full-screen call, without taking keyboard
// focus. It loads the app's `/meeting-prompt` page, is created once at startup and only ordered in
// and out after that. Where it sits comes from `panel::place`.

use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{sel, ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSEvent, NSPanel, NSScreen, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use super::panel::{place, Rect};

const LABEL: &str = "meeting-prompt";
/// Window width in points.
const WIDTH: f64 = 360.0;
/// Height of one card, and what each further card adds, in points.
const HEIGHT_ONE: f64 = 120.0;
const HEIGHT_MORE: f64 = 110.0;
/// NSPopUpMenuWindowLevel: above the menu bar and full-screen content.
const LEVEL: isize = 101;

extern "C" fn never(_this: *mut AnyObject, _cmd: Sel) -> Bool {
    Bool::NO
}

/// NSPanel that never becomes key or main, so showing or clicking it keeps focus where it was.
fn panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"MeetilyPromptPanel", NSPanel::class())
            .expect("MeetilyPromptPanel registered twice");
        unsafe {
            builder.add_method(sel!(canBecomeKeyWindow), never as extern "C" fn(_, _) -> Bool);
            builder.add_method(sel!(canBecomeMainWindow), never as extern "C" fn(_, _) -> Bool);
        }
        builder.register()
    })
}

/// Creates the hidden prompt window. Call on the main thread (the app's setup).
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("meeting-prompt".into()))
        .title("Meeting prompt")
        .inner_size(WIDTH, HEIGHT_ONE)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .focusable(false)
        .accept_first_mouse(true)
        .build();
    let window = match built {
        Ok(w) => w,
        Err(e) => return log::error!("meeting_detect: cannot create the prompt window: {}", e),
    };
    let ptr = match window.ns_window() {
        Ok(p) => p as *mut AnyObject,
        Err(e) => return log::error!("meeting_detect: no NSWindow for the prompt window: {}", e),
    };
    // SAFETY: ptr is this window's NSWindow, on the main thread. NSPanel adds no instance variables
    // to NSWindow, so the swap keeps the object's layout valid.
    unsafe {
        objc2::ffi::object_setClass(ptr, panel_class());
        let ns: &NSWindow = &*(ptr as *const NSWindow);
        ns.setStyleMask(ns.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        ns.setLevel(LEVEL);
        ns.setHidesOnDeactivate(false);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        (&*(ptr as *const NSPanel)).setFloatingPanel(true);
    }
}

/// Shows the window sized for `count` cards on the screen the pointer is on, or hides it for 0.
/// Never activates the app.
pub fn update<R: Runtime>(app: &AppHandle<R>, count: usize) {
    let handle = app.clone();
    let run = move || {
        let Some(window) = handle.get_webview_window(LABEL) else { return };
        let Ok(ptr) = window.ns_window() else { return };
        // SAFETY: the pointer is this window's NSWindow and we are on the main thread.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        if count == 0 {
            ns.orderOut(None);
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        let height = HEIGHT_ONE + HEIGHT_MORE * (count - 1) as f64;
        let (x, top) = target_point(mtm, height);
        // Cocoa's y runs up from the bottom of the primary screen; `top` runs down from its top.
        let primary_h = primary_height(mtm);
        let frame = NSRect::new(NSPoint::new(x, primary_h - top - height), NSSize::new(WIDTH, height));
        ns.setFrame_display(frame, true);
        ns.orderFrontRegardless();
    };
    if let Err(e) = app.run_on_main_thread(run) {
        log::error!("meeting_detect: cannot update the prompt window: {}", e);
    }
}

fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height)
}

/// Top-left (x, y down from the primary screen's top) for the window on the pointer's screen.
fn target_point(mtm: MainThreadMarker, height: f64) -> (f64, f64) {
    let primary_h = primary_height(mtm);
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .find(|s| {
            let f = s.frame();
            mouse.x >= f.origin.x
                && mouse.x < f.origin.x + f.size.width
                && mouse.y >= f.origin.y
                && mouse.y < f.origin.y + f.size.height
        })
        .or_else(|| screens.firstObject());
    let Some(screen) = screen else { return (0.0, 0.0) };
    let v = screen.visibleFrame();
    let visible = Rect { x: v.origin.x, y: primary_h - (v.origin.y + v.size.height), w: v.size.width, h: v.size.height };
    place(visible, WIDTH, height)
}
