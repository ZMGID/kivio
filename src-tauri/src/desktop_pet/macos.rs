//! Native Momo window for macOS. No WebView, timer, or extra process.
//!
//! `Position` is the panel's top-left corner in Cocoa global coordinates:
//! the primary display's origin is its bottom-left, and y increases upward.
//! This module stores and restores that same value.
//!
//! Right-click hides without a menu. Hide and OpenChat run on the next
//! main-thread turn, after the mouse method returns, so the view is not
//! released or a window created underneath AppKit. A double-click's first
//! click also reports `Poke`, because a platform timer is not allowed.
//! Transparent pixels pass the cursor through via `setIgnoresMouseEvents`,
//! sampled from `NSEvent.mouseLocation` whenever the parent supplies a frame.
//! An equal `Visual` does not invalidate: reduced-motion frames stay as they are.
//! Dragging keeps the panel interactive until the button is released.
//!
//! `suspended` orders the panel out while the session is locked or a display
//! is asleep, so the last frame is not left on the lock screen. The frame
//! origin is kept and `position` still reports it.

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::LazyLock;

use block2::RcBlock;
use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::{kCFBooleanFalse, kCFBooleanTrue};
use core_foundation::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::string::CFString;
use core_graphics::context::CGContext;
use core_graphics::display::CGDisplay;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use tauri::AppHandle;

use super::visual::{self, Visual, SIZE};
use super::{native_action, NativeAction, Position};

const INITIAL_STATUS: &str = "Momo · Kivio";
const DRAG_THRESHOLD: f64 = 3.0;
const WORK_AREA_MARGIN: f64 = 16.0;
const NONACTIVATING: usize = 1 << 7;
const BUFFERED: usize = 2;
const STATUS_LEVEL: isize = 25;
const ANIMATION_NONE: isize = 2;
const CONTROL_MODIFIER: usize = 1 << 18;
const JOIN_ALL_SPACES: usize = 1 << 0;
const TRANSIENT: usize = 1 << 3;
const IGNORES_CYCLE: usize = 1 << 6;
const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
const COLLECTION: usize = JOIN_ALL_SPACES | TRANSIENT | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY;
const DELIVER_IMMEDIATELY: usize = 4;

const WORKSPACE_NOTIFICATIONS: [&str; 6] = [
    "NSWorkspaceWillSleepNotification",
    "NSWorkspaceDidWakeNotification",
    "NSWorkspaceScreensDidSleepNotification",
    "NSWorkspaceScreensDidWakeNotification",
    "NSWorkspaceSessionDidResignActiveNotification",
    "NSWorkspaceSessionDidBecomeActiveNotification",
];
const LOCK_NOTIFICATIONS: [&str; 2] = ["com.apple.screenIsLocked", "com.apple.screenIsUnlocked"];

struct Press {
    origin: NSPoint,
    mouse: NSPoint,
    moved: bool,
    clicks: isize,
    right: bool,
    control: bool,
}

#[derive(Default)]
struct SuspendFlags {
    workspace_sleep: bool,
    screens_sleep: bool,
    session_inactive: bool,
    screen_locked: bool,
}

struct Speech {
    panel: id,
    label: id,
    text: String,
    visible: bool,
    origin: NSPoint,
    size: NSSize,
    dark: Option<bool>,
}

struct Pet {
    panel: id,
    view: id,
    speech: Option<Speech>,
    pointer_monitor: id,
    local_pointer_monitor: id,
    app: AppHandle,
    visual: Visual,
    press: Option<Press>,
    fractional_move: [f64; 2],
    status: String,
    flags: SuspendFlags,
    hidden_for_suspend: bool,
    /// Last value sent to `setIgnoresMouseEvents`. `false` matches a new panel.
    ignores_mouse: bool,
    workspace_observer: bool,
    distributed_observer: bool,
}

thread_local! {
    static PET: RefCell<Option<Pet>> = RefCell::new(None);
    static IN_CALLBACK: Cell<bool> = Cell::new(false);
    static SYNCING_VISIBILITY: Cell<bool> = Cell::new(false);
}

/// Panel and view classes. The names are fixed, so this does not take runtime input.
static CLASSES: LazyLock<(&'static Class, &'static Class)> = LazyLock::new(register_classes);

pub(super) fn create(
    app: &AppHandle,
    visual: &Visual,
    position: Option<Position>,
) -> Result<(), String> {
    if !on_main() {
        return Err("Momo must be created on the main thread".into());
    }
    match unsafe { objc_exception::r#try(|| create_panel(app, visual, position)) } {
        Ok(result) => result,
        Err(exc) => {
            SYNCING_VISIBILITY.with(|flag| flag.set(false));
            release_exception(exc);
            let _ = destroy_panel();
            Err("Momo panel could not be created".into())
        }
    }
}

pub(super) fn update(visual: &Visual) -> Result<(), String> {
    if !on_main() {
        return Err("Momo must be updated on the main thread".into());
    }
    match unsafe { objc_exception::r#try(|| update_visual(*visual)) } {
        Ok(result) => result,
        Err(exc) => {
            release_exception(exc);
            Err("Momo drawing failed".into())
        }
    }
}

pub(super) fn destroy() -> Option<Position> {
    if !on_main() {
        return None;
    }
    match unsafe { objc_exception::r#try(destroy_panel) } {
        Ok(position) => position,
        Err(exc) => {
            SYNCING_VISIBILITY.with(|flag| flag.set(false));
            release_exception(exc);
            None
        }
    }
}

pub(super) fn position() -> Option<Position> {
    if !on_main() {
        return None;
    }
    match unsafe { objc_exception::r#try(read_position) } {
        Ok(position) => position,
        Err(exc) => {
            release_exception(exc);
            None
        }
    }
}

pub(super) fn environment() -> super::behavior::Environment {
    let mut environment = super::behavior::Environment::default();
    if !on_main() {
        return environment;
    }
    guard(|| {
        PET.with(|cell| {
            let mut slot = cell.borrow_mut();
            let Some(pet) = slot.as_mut() else { return };
            let cursor = mouse_location();
            let frame: NSRect = unsafe { msg_send![pet.panel, frame] };
            environment.cursor =
                Some([cursor.x - frame.origin.x, frame.origin.y + SIZE - cursor.y]);
            environment.pressed = pet.press.is_some();
            if environment.pressed {
                pet.fractional_move = [0.0; 2];
            }
            environment.dragging = pet.press.as_ref().is_some_and(|press| press.moved);
            if let Some(screen) =
                screen_containing(frame.origin.x + SIZE / 2.0, frame.origin.y + SIZE / 2.0)
                    .or_else(fallback_screen)
            {
                if let Ok(work) = visible_frame(screen) {
                    environment.floor_distance = (frame.origin.y - work.origin.y).max(0.0);
                    environment.horizontal_room = [
                        (frame.origin.x - work.origin.x).max(0.0),
                        (work.origin.x + work.size.width - SIZE - frame.origin.x).max(0.0),
                    ];
                }
            }
        });
    });
    environment
}

pub(super) fn move_by(delta: [f64; 2]) -> Result<(), String> {
    if delta == [0.0, 0.0] {
        return Ok(());
    }
    if !on_main() {
        return Err("Momo must move on the main thread".into());
    }
    match unsafe {
        objc_exception::r#try(|| {
            // Release the cell before AppKit dispatches movement notifications.
            let (panel, pixels) = PET
                .with(|cell| {
                    let mut slot = cell.borrow_mut();
                    let pet = slot.as_mut()?;
                    Some((
                        pet.panel,
                        super::behavior::pixel_motion(delta, 1.0, &mut pet.fractional_move),
                    ))
                })
                .ok_or_else(|| "Momo is not open".to_string())?;
            if pixels == [0, 0] {
                return Ok(());
            }
            let frame: NSRect = msg_send![panel, frame];
            let origin = clamp_origin(NSPoint::new(
                frame.origin.x + pixels[0] as f64,
                frame.origin.y - pixels[1] as f64,
            ));
            let _: () = msg_send![panel, setFrameOrigin: origin];
            Ok(())
        })
    } {
        Ok(result) => result,
        Err(exception) => {
            release_exception(exception);
            Err("Momo could not move".into())
        }
    }
}

pub(super) fn suspended() -> bool {
    if !on_main() {
        // Drawing from the wrong thread is worse than skipping a frame.
        return true;
    }
    match unsafe { objc_exception::r#try(|| refresh_visibility(false)) } {
        Ok(value) => value,
        Err(exc) => {
            SYNCING_VISIBILITY.with(|flag| flag.set(false));
            release_exception(exc);
            true
        }
    }
}

/// Title, tooltip, and accessibility label. The parent calls this only when
/// the logical status changes; the stored text skips a repeated native update.
/// Must not re-enter `native_action`: the parent can be holding its state lock.
pub(super) fn set_status(text: &str) {
    if !on_main() {
        return;
    }
    let targets = PET.with(|cell| {
        let pet = cell.borrow();
        let pet = pet.as_ref()?;
        if pet.status == text {
            return None;
        }
        Some((pet.panel, pet.view))
    });
    let Some((panel, view)) = targets else {
        return;
    };
    let applied = match unsafe { objc_exception::r#try(|| apply_status(panel, view, text)) } {
        Ok(applied) => applied,
        Err(exc) => {
            release_exception(exc);
            false
        }
    };
    if !applied {
        eprintln!("[momo] status label was not applied");
        return;
    }
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            if pet.status != text {
                pet.status = text.to_string();
            }
        }
    });
}

/// A non-activating, click-through native label, owned by the pet panel.
pub(super) fn set_speech(text: Option<&str>) {
    if !on_main() {
        return;
    }
    let result = unsafe {
        objc_exception::r#try(|| {
            PET.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(pet) = slot.as_mut() else { return; };
            let Some(text) = text.filter(|_| !pet.hidden_for_suspend) else {
                if let Some(speech) = &mut pet.speech {
                    if speech.visible {
                        let _: () = msg_send![speech.panel, orderOut: nil];
                        speech.visible = false;
                    }
                }
                return;
            };
            let dark = super::speech_dark(&pet.app);
            if pet.speech.is_none() {
                let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 34.0));
                let Ok(panel) = init_panel(class!(NSPanel), frame) else { return; };
                let _: () = msg_send![panel, setCollectionBehavior: COLLECTION];
                let _: () = msg_send![panel, setLevel: STATUS_LEVEL];
                let _: () = msg_send![panel, setHidesOnDeactivate: NO];
                let _: () = msg_send![panel, setOpaque: NO];
                let clear: id = msg_send![class!(NSColor), clearColor];
                let _: () = msg_send![panel, setBackgroundColor: clear];
                let _: () = msg_send![panel, setHasShadow: YES];
                let _: () = msg_send![panel, setIgnoresMouseEvents: YES];
                let _: () = msg_send![panel, setAnimationBehavior: ANIMATION_NONE];
                let content: id = msg_send![class!(NSView), alloc];
                let content: id = msg_send![content, initWithFrame: frame];
                let _: () = msg_send![content, setWantsLayer: YES];
                let layer: id = msg_send![content, layer];
                let _: () = msg_send![layer, setCornerRadius: visual::SPEECH_RADIUS];
                let _: () = msg_send![layer, setBorderWidth: 0.75_f64];
                let _: () = msg_send![layer, setMasksToBounds: YES];
                let _: () = msg_send![panel, setContentView: content];
                let _: () = msg_send![content, release];
                let string = ns_string("");
                let label: id = msg_send![class!(NSTextField), wrappingLabelWithString: string];
                let font: id = msg_send![class!(NSFont), systemFontOfSize: visual::SPEECH_FONT_SIZE];
                let _: () = msg_send![label, setFont: font];
                let _: () = msg_send![content, addSubview: label];
                let _: () = msg_send![pet.panel, addChildWindow: panel ordered: 1_isize];
                pet.speech = Some(Speech { panel, label, text: String::new(), visible: false,
                    origin: NSPoint::new(f64::NAN, f64::NAN), size: NSSize::new(100.0, 34.0), dark: None });
            }
            let speech = pet.speech.as_mut().unwrap();
            if speech.dark != Some(dark) {
                let palette = visual::speech_palette(dark);
                let content: id = msg_send![speech.panel, contentView];
                let layer: id = msg_send![content, layer];
                let background = speech_color(palette.background);
                let border = speech_color(palette.border);
                let background_cg: *const c_void = msg_send![background, CGColor];
                let border_cg: *const c_void = msg_send![border, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: background_cg];
                let _: () = msg_send![layer, setBorderColor: border_cg];
                let _: () = msg_send![speech.label, setTextColor: speech_color(palette.foreground)];
                speech.dark = Some(dark);
            }
            if speech.text != text {
                let string = ns_string(text);
                let _: () = msg_send![speech.label, setStringValue: string];
                let _: () = msg_send![speech.panel, setTitle: string];
                let font: id = msg_send![speech.label, font];
                let attrs: id = msg_send![class!(NSDictionary), dictionaryWithObject: font forKey: ns_string("NSFont")];
                let natural: NSSize = msg_send![string, sizeWithAttributes: attrs];
                let width = (natural.width.ceil() + 4.0).clamp(40.0, visual::SPEECH_MAX_WIDTH);
                let cell: id = msg_send![speech.label, cell];
                let measured: NSSize = msg_send![cell, cellSizeForBounds:
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 1000.0))];
                let height = measured.height.ceil().max(16.0);
                speech.size = NSSize::new(width + visual::SPEECH_PADDING_X * 2.0,
                    height + visual::SPEECH_PADDING_Y * 2.0);
                let _: () = msg_send![speech.panel, setContentSize: speech.size];
                let _: () = msg_send![speech.label, setFrame:
                    NSRect::new(NSPoint::new(visual::SPEECH_PADDING_X, visual::SPEECH_PADDING_Y), NSSize::new(width, height))];
                speech.text.clear();
                speech.text.push_str(text);
            }
            let pet_frame: NSRect = msg_send![pet.panel, frame];
            let screen: id = msg_send![pet.panel, screen];
            if screen == nil { return; }
            let work: NSRect = msg_send![screen, visibleFrame];
            let x = (pet_frame.origin.x + 84.0 - speech.size.width * 0.5)
                .clamp(work.origin.x, (work.origin.x + work.size.width - speech.size.width).max(work.origin.x));
            let above = pet_frame.origin.y + 108.0;
            let y = if above + speech.size.height <= work.origin.y + work.size.height {
                above
            } else { pet_frame.origin.y + 32.0 - speech.size.height };
            let y = y.clamp(work.origin.y, (work.origin.y + work.size.height - speech.size.height).max(work.origin.y));
            if speech.origin.x != x || speech.origin.y != y {
                speech.origin = NSPoint::new(x, y);
                let _: () = msg_send![speech.panel, setFrameOrigin: speech.origin];
            }
            if !speech.visible {
                let _: () = msg_send![speech.panel, orderFrontRegardless];
                speech.visible = true;
            }
        });
        })
    };
    if let Err(exception) = result {
        release_exception(exception);
    }
}

unsafe fn speech_color(color: visual::Color) -> id {
    msg_send![class!(NSColor), colorWithSRGBRed: color.r green: color.g blue: color.b alpha: color.a]
}

pub(super) fn reduced_motion() -> bool {
    if !on_main() {
        return false;
    }
    match unsafe { objc_exception::r#try(read_reduced_motion) } {
        Ok(value) => value,
        Err(exc) => {
            release_exception(exc);
            false
        }
    }
}

fn create_panel(
    app: &AppHandle,
    visual: &Visual,
    position: Option<Position>,
) -> Result<(), String> {
    if PET.with(|cell| cell.borrow().is_some()) {
        return update_visual(*visual);
    }
    let (panel_class, view_class) = classes();
    let (x, top) = match position {
        Some(position) if position.x.is_finite() && position.y.is_finite() => {
            clamp_top_left(position.x, position.y)?
        }
        _ => default_top_left()?,
    };
    let frame = NSRect::new(NSPoint::new(x, top - SIZE), NSSize::new(SIZE, SIZE));
    let panel = init_panel(panel_class, frame)?;
    PET.with(|cell| {
        *cell.borrow_mut() = Some(Pet {
            panel,
            view: nil,
            speech: None,
            pointer_monitor: nil,
            local_pointer_monitor: nil,
            app: app.clone(),
            visual: *visual,
            press: None,
            fractional_move: [0.0; 2],
            status: String::new(),
            flags: SuspendFlags::default(),
            // True until the panel is actually ordered in, so a failed first show is retried.
            hidden_for_suspend: true,
            ignores_mouse: false,
            workspace_observer: false,
            distributed_observer: false,
        });
    });
    if let Err(error) = finish_panel(view_class, panel) {
        let _ = destroy_panel();
        return Err(error);
    }
    Ok(())
}

fn finish_panel(view_class: &Class, panel: id) -> Result<(), String> {
    configure_panel(panel);
    let view = install_view(view_class, panel)?;
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            pet.view = view;
        }
    });
    configure_layer(view);
    if !apply_status(panel, view, INITIAL_STATUS) {
        return Err("Momo accessibility label could not be set".into());
    }
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            pet.status = INITIAL_STATUS.to_string();
        }
    });
    let identifier = ns_string("kivio.momo");
    if identifier != nil {
        unsafe {
            let _: () = msg_send![identifier, autorelease];
            set_identifier(panel, identifier);
            set_identifier(view, identifier);
        }
    }
    install_observers(panel)?;
    install_pointer_monitor()?;
    let _ = refresh_visibility(true);
    Ok(())
}

fn init_panel(class: &Class, frame: NSRect) -> Result<id, String> {
    unsafe {
        let allocated: id = msg_send![class, alloc];
        let panel: id = msg_send![allocated, initWithContentRect: frame styleMask: NONACTIVATING backing: BUFFERED defer: NO];
        if panel == nil {
            return Err("Momo panel could not be created".into());
        }
        // alloc/init is the +1 we release in destroy. close must not release it.
        let _: () = msg_send![panel, setReleasedWhenClosed: NO];
        Ok(panel)
    }
}

fn configure_panel(panel: id) {
    unsafe {
        let _: () = msg_send![panel, setCollectionBehavior: COLLECTION];
        let _: () = msg_send![panel, setLevel: STATUS_LEVEL];
        let _: () = msg_send![panel, setHidesOnDeactivate: NO];
        let _: () = msg_send![panel, setOpaque: NO];
        let clear: id = msg_send![class!(NSColor), clearColor];
        if clear != nil {
            let _: () = msg_send![panel, setBackgroundColor: clear];
        }
        let _: () = msg_send![panel, setHasShadow: NO];
        let _: () = msg_send![panel, setExcludedFromWindowsMenu: YES];
        let _: () = msg_send![panel, setMovable: NO];
        let _: () = msg_send![panel, setAcceptsMouseMovedEvents: YES];
        let _: () = msg_send![panel, setAnimationBehavior: ANIMATION_NONE];
        let _: () = msg_send![panel, setRestorable: NO];
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: YES];
        let _: () = msg_send![panel, setAlphaValue: 1.0_f64];
        let size = NSSize::new(SIZE, SIZE);
        let _: () = msg_send![panel, setContentMinSize: size];
        let _: () = msg_send![panel, setContentMaxSize: size];
    }
}

fn install_view(class: &Class, panel: id) -> Result<id, String> {
    let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE, SIZE));
    unsafe {
        let allocated: id = msg_send![class, alloc];
        let view: id = msg_send![allocated, initWithFrame: bounds];
        if view == nil {
            return Err("Momo view could not be created".into());
        }
        // The panel retains the content view. Our +1 drains with the pool.
        let _: () = msg_send![view, autorelease];
        let _: () = msg_send![panel, setContentView: view];
        let selector = sel!(setAccessibilityElement:);
        let responds: BOOL = msg_send![view, respondsToSelector: selector];
        if responds != NO {
            let _: () = msg_send![view, setAccessibilityElement: YES];
        }
        Ok(view)
    }
}

fn configure_layer(view: id) {
    unsafe {
        let _: () = msg_send![view, setWantsLayer: YES];
        let layer: id = msg_send![view, layer];
        if layer == nil {
            return;
        }
        let _: () = msg_send![layer, setOpaque: NO];
        let clear: id = msg_send![class!(NSColor), clearColor];
        if clear == nil {
            return;
        }
        let color: *mut c_void = msg_send![clear, CGColor];
        if !color.is_null() {
            let _: () = msg_send![layer, setBackgroundColor: color];
        }
    }
}

fn update_visual(visual: Visual) -> Result<(), String> {
    let (view, changed) = PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let pet = slot
            .as_mut()
            .ok_or_else(|| "Momo is not open".to_string())?;
        let changed = pet.visual != visual;
        if changed {
            pet.visual = visual;
        }
        if pet.view == nil {
            return Err("Momo view is missing".to_string());
        }
        Ok((pet.view, changed))
    })?;
    // The cursor can enter or leave the body while the picture stays put.
    sync_passthrough();
    if changed {
        unsafe {
            let _: () = msg_send![view, setNeedsDisplay: YES];
        }
    }
    Ok(())
}

fn destroy_panel() -> Option<Position> {
    let pet = PET.with(|cell| cell.borrow_mut().take())?;
    let panel = pet.panel;
    if panel == nil {
        return None;
    }
    let position = top_left(panel);
    remove_observers(panel, pet.workspace_observer, pet.distributed_observer);
    unsafe {
        if let Some(speech) = pet.speech {
            let _: () = msg_send![panel, removeChildWindow: speech.panel];
            let _: () = msg_send![speech.panel, orderOut: nil];
            let _: () = msg_send![speech.panel, close];
            let _: () = msg_send![speech.panel, release];
        }
        for monitor in [pet.pointer_monitor, pet.local_pointer_monitor] {
            if monitor != nil {
                let _: () = msg_send![class!(NSEvent), removeMonitor: monitor];
                let _: () = msg_send![monitor, release];
            }
        }
        let _: () = msg_send![panel, orderOut: nil];
        let _: () = msg_send![panel, setReleasedWhenClosed: NO];
        let _: () = msg_send![panel, close];
        // A click handler is still on this view's stack. Release after it returns.
        if IN_CALLBACK.with(|flag| flag.get()) {
            let _: () = msg_send![panel, autorelease];
        } else {
            let _: () = msg_send![panel, release];
        }
    }
    position
}

fn read_position() -> Option<Position> {
    let panel = PET.with(|cell| cell.borrow().as_ref().map(|pet| pet.panel))?;
    if panel == nil {
        None
    } else {
        top_left(panel)
    }
}

fn apply_status(panel: id, view: id, text: &str) -> bool {
    let title = ns_string(text);
    if title == nil || panel == nil {
        return false;
    }
    unsafe {
        let _: () = msg_send![title, autorelease];
        let _: () = msg_send![panel, setTitle: title];
        set_object_if_responds(panel, sel!(setAccessibilityLabel:), title);
        if view != nil {
            set_object_if_responds(view, sel!(setToolTip:), title);
            set_object_if_responds(view, sel!(setAccessibilityLabel:), title);
        }
    }
    true
}

fn set_identifier(object: id, identifier: id) {
    set_object_if_responds(object, sel!(setAccessibilityIdentifier:), identifier);
}

fn set_object_if_responds(object: id, selector: Sel, value: id) {
    if object == nil {
        return;
    }
    unsafe {
        let responds: BOOL = msg_send![object, respondsToSelector: selector];
        if responds != NO {
            let _: id = msg_send![object, performSelector: selector withObject: value];
        }
    }
}

fn read_reduced_motion() -> bool {
    unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace == nil {
            return false;
        }
        let selector = sel!(accessibilityDisplayShouldReduceMotion);
        let responds: BOOL = msg_send![workspace, respondsToSelector: selector];
        if responds == NO {
            return false;
        }
        let value: BOOL = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        value != NO
    }
}

fn classes() -> (&'static Class, &'static Class) {
    *CLASSES
}

fn register_classes() -> (&'static Class, &'static Class) {
    (register_panel_class(), register_view_class())
}

fn register_panel_class() -> &'static Class {
    let mut declaration =
        ClassDecl::new("KivioMomoPanel", class!(NSPanel)).expect("declare KivioMomoPanel");
    unsafe {
        declaration.add_method(
            sel!(canBecomeKeyWindow),
            can_become_key as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(canBecomeMainWindow),
            can_become_main as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(_isNonactivatingPanel),
            is_nonactivating as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(momoNotification:),
            on_notification as extern "C" fn(&Object, Sel, id),
        );
    }
    declaration.register()
}

fn register_view_class() -> &'static Class {
    let mut declaration =
        ClassDecl::new("KivioMomoView", class!(NSView)).expect("declare KivioMomoView");
    unsafe {
        declaration.add_method(
            sel!(isOpaque),
            is_opaque as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        declaration.add_method(
            sel!(acceptsFirstResponder),
            accepts_first_responder as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(drawRect:),
            draw_rect as extern "C" fn(&Object, Sel, NSRect),
        );
        declaration.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseDragged:),
            mouse_dragged as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(sel!(mouseUp:), mouse_up as extern "C" fn(&Object, Sel, id));
        declaration.add_method(
            sel!(rightMouseDown:),
            right_mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(rightMouseDragged:),
            right_mouse_dragged as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(rightMouseUp:),
            right_mouse_up as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(menuForEvent:),
            menu_for_event as extern "C" fn(&Object, Sel, id) -> id,
        );
    }
    declaration.register()
}

extern "C" fn can_become_key(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn can_become_main(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn is_nonactivating(_this: &Object, _cmd: Sel) -> BOOL {
    YES
}

extern "C" fn is_opaque(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn accepts_first_mouse(_this: &Object, _cmd: Sel, _event: id) -> BOOL {
    YES
}

extern "C" fn accepts_first_responder(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn menu_for_event(_this: &Object, _cmd: Sel, _event: id) -> id {
    nil
}

extern "C" fn draw_rect(this: &Object, _cmd: Sel, _dirty: NSRect) {
    let visual = PET.with(|cell| cell.borrow().as_ref().map(|pet| pet.visual));
    let Some(visual) = visual else {
        return;
    };
    guard(|| paint(this, visual));
}

extern "C" fn mouse_down(_this: &Object, _cmd: Sel, event: id) {
    guard(|| during_callback(|| begin_press(event, false)));
}

extern "C" fn mouse_dragged(_this: &Object, _cmd: Sel, _event: id) {
    guard(|| during_callback(|| drag(false)));
}

extern "C" fn mouse_up(_this: &Object, _cmd: Sel, event: id) {
    guard(|| during_callback(|| end_press(event, false)));
}

extern "C" fn right_mouse_down(_this: &Object, _cmd: Sel, event: id) {
    guard(|| during_callback(|| begin_press(event, true)));
}

extern "C" fn right_mouse_dragged(_this: &Object, _cmd: Sel, _event: id) {
    guard(|| during_callback(|| drag(true)));
}

extern "C" fn right_mouse_up(_this: &Object, _cmd: Sel, event: id) {
    guard(|| during_callback(|| end_press(event, true)));
}

extern "C" fn on_notification(_this: &Object, _cmd: Sel, note: id) {
    if !on_main() {
        return;
    }
    guard(|| record_notification(note));
    guard(|| {
        let _ = refresh_visibility(false);
    });
}

fn begin_press(event: id, right: bool) {
    if event == nil {
        return;
    }
    let clicks: isize = unsafe { msg_send![event, clickCount] };
    let modifiers: usize = unsafe { msg_send![event, modifierFlags] };
    let mouse = mouse_location();
    PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(pet) = slot.as_mut() else {
            return;
        };
        if pet.panel == nil {
            return;
        }
        let frame: NSRect = unsafe { msg_send![pet.panel, frame] };
        pet.press = Some(Press {
            origin: frame.origin,
            mouse,
            moved: false,
            clicks,
            right,
            control: modifiers & CONTROL_MODIFIER != 0,
        });
    });
    if let Some(panel) = current_panel() {
        unsafe {
            let _: () = msg_send![panel, setIgnoresMouseEvents: NO];
        }
        PET.with(|cell| {
            if let Some(pet) = cell.borrow_mut().as_mut() {
                if pet.panel == panel {
                    pet.ignores_mouse = false;
                }
            }
        });
    }
}

fn drag(right: bool) {
    if right {
        return;
    }
    let mouse = mouse_location();
    let plan = PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let pet = slot.as_mut()?;
        let press = pet.press.as_mut()?;
        if press.right || press.control || pet.panel == nil {
            return None;
        }
        let dx = mouse.x - press.mouse.x;
        let dy = mouse.y - press.mouse.y;
        if dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD {
            press.moved = true;
        }
        if !press.moved {
            return None;
        }
        Some((
            pet.panel,
            NSPoint::new(press.origin.x + dx, press.origin.y + dy),
        ))
    });
    let Some((panel, origin)) = plan else {
        return;
    };
    let origin = clamp_origin(origin);
    unsafe {
        let _: () = msg_send![panel, setFrameOrigin: origin];
    }
}

fn end_press(event: id, right: bool) {
    let modifiers: usize = if event == nil {
        0
    } else {
        unsafe { msg_send![event, modifierFlags] }
    };
    let clicks: isize = if event == nil {
        1
    } else {
        unsafe { msg_send![event, clickCount] }
    };
    let action = PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let pet = slot.as_mut()?;
        let press = pet.press.take()?;
        let hide = right || press.right || press.control || modifiers & CONTROL_MODIFIER != 0;
        if hide {
            return Some(NativeAction::Hide);
        }
        if press.moved {
            return top_left(pet.panel).map(NativeAction::PositionChanged);
        }
        if clicks.max(press.clicks) >= 2 {
            return Some(NativeAction::OpenChat);
        }
        Some(NativeAction::Poke)
    });
    match action {
        Some(action @ (NativeAction::Hide | NativeAction::OpenChat)) => {
            // The panel is still alive. Refresh hit-testing, then leave this
            // mouse method before Hide releases it or OpenChat creates a window.
            sync_passthrough();
            defer_action(action);
        }
        Some(action) => {
            deliver(action);
            sync_passthrough();
        }
        None => sync_passthrough(),
    }
}

fn defer_action(action: NativeAction) {
    let app = PET.with(|cell| cell.borrow().as_ref().map(|pet| pet.app.clone()));
    let Some(app) = app else {
        return;
    };
    // Posted to the event loop, so this returns before Hide destroys the
    // panel or OpenChat creates windows.
    if app
        .clone()
        .run_on_main_thread(move || {
            if PET.with(|cell| cell.borrow().is_none()) {
                return;
            }
            native_action(&app, action);
        })
        .is_err()
    {
        eprintln!("[momo] click was not delivered");
    }
}

fn deliver(action: NativeAction) {
    let app = PET.with(|cell| cell.borrow().as_ref().map(|pet| pet.app.clone()));
    let Some(app) = app else {
        return;
    };
    let already = IN_CALLBACK.with(|flag| flag.replace(true));
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            if !self.0 {
                IN_CALLBACK.with(|flag| flag.set(false));
            }
        }
    }
    let _reset = Reset(already);
    native_action(&app, action);
}

fn install_pointer_monitor() -> Result<(), String> {
    // Motion updates hit testing before the next click, independently of paint FPS.
    // Global monitors exclude this app; a local monitor covers its own panel.
    let handler = RcBlock::new(|_: *mut objc2::runtime::AnyObject| {
        if on_main() {
            guard(sync_passthrough);
        }
    });
    let pointer = &*handler as *const _ as *const c_void;
    unsafe {
        let monitor: id = msg_send![class!(NSEvent), addGlobalMonitorForEventsMatchingMask: (1_usize << 5) handler: pointer];
        if monitor == nil {
            return Err("Momo pointer monitor could not be installed".into());
        }
        let _: id = msg_send![monitor, retain];
        PET.with(|cell| {
            if let Some(pet) = cell.borrow_mut().as_mut() {
                pet.pointer_monitor = monitor;
            }
        });
    }
    let local = RcBlock::new(|event: *mut objc2::runtime::AnyObject| {
        if on_main() {
            guard(sync_passthrough);
        }
        event
    });
    let pointer = &*local as *const _ as *const c_void;
    unsafe {
        let monitor: id = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: (1_usize << 5) handler: pointer];
        if monitor == nil {
            return Err("Momo local pointer monitor could not be installed".into());
        }
        let _: id = msg_send![monitor, retain];
        PET.with(|cell| {
            if let Some(pet) = cell.borrow_mut().as_mut() {
                pet.local_pointer_monitor = monitor;
            }
        });
    }
    Ok(())
}

fn sync_passthrough() {
    let snapshot = PET.with(|cell| {
        let pet = cell.borrow();
        let pet = pet.as_ref()?;
        if pet.panel == nil {
            return None;
        }
        Some((
            pet.panel,
            pet.visual,
            pet.press.is_some(),
            pet.ignores_mouse,
        ))
    });
    let Some((panel, visual, pressed, previous)) = snapshot else {
        return;
    };
    let ignore = if pressed {
        false
    } else {
        match cursor_visual_point(panel) {
            Some((x, y)) => !visual.contains(x, y),
            None => true,
        }
    };
    if previous == ignore {
        return;
    }
    unsafe {
        let _: () = msg_send![panel, setIgnoresMouseEvents: objc_bool(ignore)];
    }
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            if pet.panel == panel {
                pet.ignores_mouse = ignore;
            }
        }
    });
}

fn paint(view: &Object, visual: Visual) {
    let bounds: NSRect = unsafe { msg_send![view, bounds] };
    let width = bounds.size.width;
    let height = bounds.size.height;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let current: id = unsafe { msg_send![class!(NSGraphicsContext), currentContext] };
    if current == nil {
        return;
    }
    let raw: *mut core_graphics::sys::CGContext = unsafe { msg_send![current, CGContext] };
    if raw.is_null() {
        return;
    }
    // Retain the AppKit context for this draw and release it with `ctx`.
    let ctx = unsafe { CGContext::from_existing_context_ptr(raw) };
    ctx.save();
    // AppKit's context is bottom-left, y up, in points (backing scale included).
    // Concatenate a top-left, y-down space in the 128-point visual.
    // User (x, y) becomes (x * width / SIZE, height - y * height / SIZE).
    ctx.translate(0.0, height);
    ctx.scale(width / SIZE, -height / SIZE);
    ctx.set_should_antialias(true);
    ctx.clear_rect(CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(SIZE, SIZE),
    ));
    super::visual::paint_macos(&ctx, &visual);
    ctx.restore();
}

fn install_observers(panel: id) -> Result<(), String> {
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            pet.workspace_observer = true;
        }
    });
    let workspace = workspace_center()?;
    for name in WORKSPACE_NOTIFICATIONS {
        add_observer(workspace, panel, name, None)?;
    }
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            pet.distributed_observer = true;
        }
    });
    let distributed = distributed_center()?;
    for name in LOCK_NOTIFICATIONS {
        add_observer(distributed, panel, name, Some(DELIVER_IMMEDIATELY))?;
    }
    Ok(())
}

fn workspace_center() -> Result<id, String> {
    unsafe {
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace == nil {
            return Err("NSWorkspace is unavailable".into());
        }
        let center: id = msg_send![workspace, notificationCenter];
        if center == nil {
            Err("NSWorkspace notification center is unavailable".into())
        } else {
            Ok(center)
        }
    }
}

fn distributed_center() -> Result<id, String> {
    unsafe {
        let center: id = msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
        if center == nil {
            Err("Distributed notification center is unavailable".into())
        } else {
            Ok(center)
        }
    }
}

fn add_observer(
    center: id,
    panel: id,
    name: &str,
    suspension: Option<usize>,
) -> Result<(), String> {
    let label = ns_string(name);
    if label == nil {
        return Err(format!("Could not register {name}"));
    }
    unsafe {
        let _: () = msg_send![label, autorelease];
        if let Some(behavior) = suspension {
            let _: () = msg_send![center, addObserver: panel selector: sel!(momoNotification:) name: label object: nil suspensionBehavior: behavior];
        } else {
            let _: () = msg_send![center, addObserver: panel selector: sel!(momoNotification:) name: label object: nil];
        }
    }
    Ok(())
}

fn remove_observers(panel: id, workspace: bool, distributed: bool) {
    unsafe {
        if workspace {
            if let Ok(center) = workspace_center() {
                let _: () = msg_send![center, removeObserver: panel];
            }
        }
        if distributed {
            if let Ok(center) = distributed_center() {
                let _: () = msg_send![center, removeObserver: panel];
            }
        }
    }
}

fn record_notification(note: id) {
    if note == nil {
        return;
    }
    let name: id = unsafe { msg_send![note, name] };
    let change = if ns_eq(name, "NSWorkspaceWillSleepNotification") {
        Some(SuspendChange::Workspace(true))
    } else if ns_eq(name, "NSWorkspaceDidWakeNotification") {
        Some(SuspendChange::Workspace(false))
    } else if ns_eq(name, "NSWorkspaceScreensDidSleepNotification") {
        Some(SuspendChange::Screens(true))
    } else if ns_eq(name, "NSWorkspaceScreensDidWakeNotification") {
        Some(SuspendChange::Screens(false))
    } else if ns_eq(name, "NSWorkspaceSessionDidResignActiveNotification") {
        Some(SuspendChange::Session(true))
    } else if ns_eq(name, "NSWorkspaceSessionDidBecomeActiveNotification") {
        Some(SuspendChange::Session(false))
    } else if ns_eq(name, "com.apple.screenIsLocked") {
        Some(SuspendChange::Locked(true))
    } else if ns_eq(name, "com.apple.screenIsUnlocked") {
        Some(SuspendChange::Locked(false))
    } else {
        None
    };
    let Some(change) = change else {
        return;
    };
    PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(pet) = slot.as_mut() else {
            return;
        };
        match change {
            SuspendChange::Workspace(asleep) => pet.flags.workspace_sleep = asleep,
            SuspendChange::Screens(asleep) => pet.flags.screens_sleep = asleep,
            SuspendChange::Session(inactive) => pet.flags.session_inactive = inactive,
            SuspendChange::Locked(locked) => pet.flags.screen_locked = locked,
        }
    });
}

enum SuspendChange {
    Workspace(bool),
    Screens(bool),
    Session(bool),
    Locked(bool),
}

fn refresh_visibility(show_if_active: bool) -> bool {
    let display_asleep = CGDisplay::main().is_asleep();
    let locked = session_screen_locked();
    let decision = PET.with(|cell| {
        let mut slot = cell.borrow_mut();
        let pet = slot.as_mut()?;
        let suspended = reconcile(&mut pet.flags, display_asleep, locked);
        if suspended {
            pet.press = None;
        }
        Some((pet.panel, pet.view, pet.hidden_for_suspend, suspended))
    });
    let Some((panel, view, hidden, suspended)) = decision else {
        return display_asleep || matches!(locked, Some(true));
    };
    if SYNCING_VISIBILITY.with(|flag| flag.replace(true)) {
        return suspended;
    }
    struct ClearSync;
    impl Drop for ClearSync {
        fn drop(&mut self) {
            SYNCING_VISIBILITY.with(|flag| flag.set(false));
        }
    }
    let _clear = ClearSync;
    if panel == nil {
        return suspended;
    }
    if suspended {
        if !hidden {
            unsafe {
                let _: () = msg_send![panel, orderOut: nil];
            }
            mark_hidden(true);
        }
    } else if hidden || show_if_active {
        sync_passthrough();
        unsafe {
            let _: () = msg_send![panel, orderFrontRegardless];
        }
        if view != nil {
            unsafe {
                let _: () = msg_send![view, display];
            }
        }
        mark_hidden(false);
    }
    suspended
}

fn reconcile(flags: &mut SuspendFlags, display_asleep: bool, locked: Option<bool>) -> bool {
    if locked == Some(true) {
        flags.screen_locked = true;
    }
    // A missed wake or unlock must not leave the pet hidden forever.
    // Session resign stays until its own notification: it is not the lock flag.
    if !display_asleep && locked == Some(false) {
        flags.workspace_sleep = false;
        flags.screens_sleep = false;
        flags.screen_locked = false;
    }
    flags.workspace_sleep
        || flags.screens_sleep
        || flags.session_inactive
        || locked.unwrap_or(flags.screen_locked)
        || display_asleep
}

fn session_screen_locked() -> Option<bool> {
    let dictionary = session_dictionary()?;
    unsafe {
        let value = CFDictionaryGetValue(dictionary, lock_key());
        let locked = if value.is_null() {
            None
        } else if value == kCFBooleanTrue as *const c_void {
            Some(true)
        } else if value == kCFBooleanFalse as *const c_void {
            Some(false)
        } else {
            None
        };
        CFRelease(dictionary as CFTypeRef);
        locked
    }
}

fn session_dictionary() -> Option<CFDictionaryRef> {
    static SYMBOL: LazyLock<usize> = LazyLock::new(|| unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            b"CGSessionCopyCurrentDictionary\0".as_ptr() as *const c_char,
        ) as usize
    });
    let symbol = *SYMBOL;
    if symbol == 0 {
        return None;
    }
    let function: unsafe extern "C" fn() -> CFDictionaryRef =
        unsafe { std::mem::transmute(symbol) };
    let dictionary = unsafe { function() };
    if dictionary.is_null() {
        None
    } else {
        Some(dictionary)
    }
}

fn lock_key() -> *const c_void {
    static KEY: LazyLock<usize> = LazyLock::new(|| {
        let text = CFString::new("CGSSessionScreenIsLocked");
        let pointer = text.as_concrete_TypeRef() as usize;
        std::mem::forget(text);
        pointer
    });
    *KEY as *const c_void
}

fn mark_hidden(hidden: bool) {
    PET.with(|cell| {
        if let Some(pet) = cell.borrow_mut().as_mut() {
            pet.hidden_for_suspend = hidden;
        }
    });
}

fn default_top_left() -> Result<(f64, f64), String> {
    let mouse = mouse_location();
    let screen = screen_containing(mouse.x, mouse.y)
        .or_else(fallback_screen)
        .ok_or_else(|| "No display available for Momo".to_string())?;
    let visible = visible_frame(screen)?;
    let x = visible.origin.x + visible.size.width - SIZE - WORK_AREA_MARGIN;
    let y = visible.origin.y + SIZE + WORK_AREA_MARGIN;
    Ok(clamp_to_visible(x, y, visible))
}

fn clamp_top_left(x: f64, y: f64) -> Result<(f64, f64), String> {
    let screen = screen_containing(x + SIZE / 2.0, y - SIZE / 2.0)
        .or_else(fallback_screen)
        .ok_or_else(|| "No display available for Momo".to_string())?;
    let visible = visible_frame(screen)?;
    Ok(clamp_to_visible(x, y, visible))
}

fn clamp_origin(origin: NSPoint) -> NSPoint {
    match clamp_top_left(origin.x, origin.y + SIZE) {
        Ok((x, top)) => NSPoint::new(x, top - SIZE),
        Err(_) => origin,
    }
}

fn clamp_to_visible(x: f64, y: f64, visible: NSRect) -> (f64, f64) {
    let min_x = visible.origin.x;
    let max_x = visible.origin.x + visible.size.width - SIZE;
    let min_y = visible.origin.y + SIZE;
    let max_y = visible.origin.y + visible.size.height;
    (
        clamp_axis(x, min_x, max_x, false),
        clamp_axis(y, min_y, max_y, true),
    )
}

fn clamp_axis(value: f64, min: f64, max: f64, prefer_max: bool) -> f64 {
    if !value.is_finite() {
        return if prefer_max { max } else { min };
    }
    if max < min {
        if prefer_max {
            max
        } else {
            min
        }
    } else {
        value.clamp(min, max)
    }
}

fn visible_frame(screen: id) -> Result<NSRect, String> {
    if screen == nil {
        return Err("No display available for Momo".into());
    }
    let visible: NSRect = unsafe { msg_send![screen, visibleFrame] };
    if visible.size.width < 1.0 || visible.size.height < 1.0 {
        return Err("Display work area is empty".into());
    }
    Ok(visible)
}

fn screen_containing(x: f64, y: f64) -> Option<id> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    unsafe {
        let screens: id = msg_send![class!(NSScreen), screens];
        if screens == nil {
            return None;
        }
        let count: usize = msg_send![screens, count];
        for index in 0..count {
            let screen: id = msg_send![screens, objectAtIndex: index];
            if screen == nil {
                continue;
            }
            let frame: NSRect = msg_send![screen, frame];
            let max_x = frame.origin.x + frame.size.width;
            let max_y = frame.origin.y + frame.size.height;
            if x >= frame.origin.x && x < max_x && y >= frame.origin.y && y < max_y {
                return Some(screen);
            }
        }
        None
    }
}

fn fallback_screen() -> Option<id> {
    unsafe {
        let main: id = msg_send![class!(NSScreen), mainScreen];
        if main != nil {
            return Some(main);
        }
        let screens: id = msg_send![class!(NSScreen), screens];
        if screens == nil {
            return None;
        }
        let count: usize = msg_send![screens, count];
        if count == 0 {
            return None;
        }
        let screen: id = msg_send![screens, objectAtIndex: 0usize];
        if screen == nil {
            None
        } else {
            Some(screen)
        }
    }
}

fn top_left(panel: id) -> Option<Position> {
    if panel == nil {
        return None;
    }
    let frame: NSRect = unsafe { msg_send![panel, frame] };
    let position = Position {
        x: frame.origin.x,
        y: frame.origin.y + frame.size.height,
    };
    if position.x.is_finite() && position.y.is_finite() {
        Some(position)
    } else {
        None
    }
}

fn cursor_visual_point(panel: id) -> Option<(f64, f64)> {
    let frame: NSRect = unsafe { msg_send![panel, frame] };
    if frame.size.width <= 0.0 || frame.size.height <= 0.0 {
        return None;
    }
    let mouse = mouse_location();
    let x = (mouse.x - frame.origin.x) * SIZE / frame.size.width;
    let y = (frame.origin.y + frame.size.height - mouse.y) * SIZE / frame.size.height;
    if x.is_finite() && y.is_finite() {
        Some((x, y))
    } else {
        None
    }
}

fn mouse_location() -> NSPoint {
    unsafe { msg_send![class!(NSEvent), mouseLocation] }
}

fn current_panel() -> Option<id> {
    PET.with(|cell| {
        let pet = cell.borrow();
        let panel = pet.as_ref()?.panel;
        if panel == nil {
            None
        } else {
            Some(panel)
        }
    })
}

fn ns_string(text: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(text) }
}

fn ns_eq(value: id, expected: &str) -> bool {
    if value == nil {
        return false;
    }
    unsafe {
        let bytes: *const c_char = msg_send![value, UTF8String];
        if bytes.is_null() {
            return false;
        }
        CStr::from_ptr(bytes).to_bytes() == expected.as_bytes()
    }
}

fn ns_utf8(value: id) -> String {
    if value == nil {
        return String::new();
    }
    unsafe {
        let bytes: *const c_char = msg_send![value, UTF8String];
        if bytes.is_null() {
            String::new()
        } else {
            CStr::from_ptr(bytes).to_string_lossy().into_owned()
        }
    }
}

fn objc_bool(value: bool) -> BOOL {
    if value {
        YES
    } else {
        NO
    }
}

fn on_main() -> bool {
    crate::windows::macos_is_main_thread()
}

fn during_callback(body: impl FnOnce()) {
    let outer = IN_CALLBACK.with(|flag| flag.replace(true));
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            IN_CALLBACK.with(|flag| flag.set(self.0));
        }
    }
    let _reset = Reset(outer);
    body();
}

fn guard(body: impl FnOnce()) {
    if let Err(exc) = unsafe { objc_exception::r#try(body) } {
        // An Objective-C exception skips Rust destructors, including the
        // visibility guard. Clear it here or later frames would never show.
        SYNCING_VISIBILITY.with(|flag| flag.set(false));
        IN_CALLBACK.with(|flag| flag.set(false));
        release_exception(exc);
    }
}

fn release_exception(exc: *mut objc_exception::Exception) {
    if exc.is_null() {
        return;
    }
    unsafe {
        let object = exc as *mut Object;
        let _ = objc_exception::r#try(|| {
            let name: id = msg_send![object, name];
            let reason: id = msg_send![object, reason];
            eprintln!(
                "[momo] NSException name={} reason={}",
                ns_utf8(name),
                ns_utf8(reason)
            );
        });
        objc::runtime::objc_release(object);
    }
}
