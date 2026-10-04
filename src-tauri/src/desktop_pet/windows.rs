//! Transparent Momo window hosted by the Tauri UI thread.
//!
//! One layered popup (`WS_EX_LAYERED` + `UpdateLayeredWindow`) shows the
//! parent's premultiplied BGRA. The memory DC and DIB exist once per pixel
//! size and are released when the window is destroyed. Empty pixels return
//! `HTTRANSPARENT`, so clicks pass through; `WS_EX_TRANSPARENT` is not used
//! because it would disable the whole pet. Only visible body and prop ink hit.
//!
//! Right-click hides immediately and does not open a context menu. A double-click
//! opens Chat; its first click still pokes, because this module does not own a timer.
//! Ordinary clicks and drags do not activate the window and it has no taskbar button.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::{self, size_of};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use tauri::AppHandle;
use windows::core::{w, BOOL, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, SIZE as WinSize, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetDeviceCaps,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    LOGPIXELSX, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGBQUAD,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Accessibility::NotifyWinEvent;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetMessagePos, GetMessageTime,
    GetSystemMetrics, GetWindowRect, IsWindowVisible, LoadCursorW, RegisterClassExW, SendMessageW,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, UpdateLayeredWindow, CHILDID_SELF,
    CS_DBLCLKS, EVENT_OBJECT_NAMECHANGE, HTTRANSPARENT, IDC_ARROW, MA_NOACTIVATE, MSG,
    OBJID_WINDOW, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND,
    PBT_APMSUSPEND, SM_CXDRAG, SM_CYDRAG, SPI_GETCLIENTAREAANIMATION, SPI_GETSCREENSAVERRUNNING,
    SPI_GETWORKAREA, SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, ULW_ALPHA,
    WINDOW_STYLE, WM_CAPTURECHANGED, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_NCHITTEST, WM_POWERBROADCAST, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_USER, WM_WTSSESSION_CHANGE,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use super::visual::{self, Visual, SIZE};
use super::{native_action, NativeAction, Position};

const INITIAL_STATUS: &str = "Momo · Kivio";
const STATUS_CAPACITY: usize = 256;
const TOOLTIP_FLAGS: u32 = 0x0001 | 0x0100; // TTF_IDISHWND | TTF_TRANSPARENT
const TOOLTIP_STYLE: u32 = 0x01 | 0x02; // TTS_ALWAYSTIP | TTS_NOPREFIX
const TTM_ADDTOOLW: u32 = WM_USER + 50;
const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;
const TTM_SETMAXTIPWIDTH: u32 = WM_USER + 24;
const TTM_RELAYEVENT: u32 = WM_USER + 7;
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

thread_local! {
    static PET: RefCell<Option<Pet>> = RefCell::new(None);
    // UpdateLayeredWindow can synchronously send WM_DPICHANGED while PET is
    // borrowed. Keep the latest notification until redraw can consume it.
    static PENDING_DPI: Cell<u32> = const { Cell::new(0) };
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
}

struct Speech {
    hwnd: HWND,
    text: String,
    wide: Vec<u16>,
    visible: bool,
    origin: (i32, i32),
}

impl Drop for Speech {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct Pet {
    hwnd: HWND,
    tooltip: HWND,
    speech: Option<Speech>,
    app: AppHandle,
    visual: Visual,
    origin_x: i32,
    origin_y: i32,
    dpi: u32,
    pixel_size: i32,
    surface: Surface,
    captured: bool,
    dragged: bool,
    swallow_up: bool,
    down_cursor: POINT,
    down_origin: (i32, i32),
    fractional_move: [f64; 2],
    position_dirty: bool,
    power_suspended: bool,
    session_locked: bool,
    session_notifications: bool,
    /// Stable UTF-16 title. The tooltip may retain this pointer, so the pet is not moved
    /// after the tooltip is created and the buffer is never reallocated.
    status: [u16; STATUS_CAPACITY],
    status_len: usize,
}

pub(super) fn create(
    app: &AppHandle,
    visual: &Visual,
    position: Option<Position>,
) -> Result<(), String> {
    match pet_installed() {
        Some(true) => return update(visual),
        Some(false) => {}
        None => return Err("Momo window is busy".into()),
    }
    register_class()?;
    PENDING_DPI.with(|pending| pending.set(0));
    let pet = Pet::open(app, visual, position)?;
    PET.with(|slot| *slot.borrow_mut() = Some(pet));
    reveal();
    Ok(())
}

pub(super) fn update(visual: &Visual) -> Result<(), String> {
    let mut failure: Option<String> = None;
    let found = PET.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return true;
        };
        let Some(pet) = slot.as_mut() else {
            return false;
        };
        if let Err(error) = pet.redraw(visual) {
            failure = Some(error);
        }
        true
    });
    match failure {
        Some(error) => Err(error),
        None if found => Ok(()),
        None => Err("Momo window is not open".into()),
    }
}

pub(super) fn destroy() -> Option<Position> {
    // Close the tooltip while the pet is still stored. It may retain the status
    // buffer pointer, which would dangle once the pet moves out of thread-local storage.
    let position = with_pet(|pet| {
        close_tooltip(pet);
        pet.screen_position()
    })?;
    let pet = PET.with(|slot| slot.try_borrow_mut().ok()?.take())?;
    drop(pet);
    PENDING_DPI.with(|pending| pending.set(0));
    Some(position)
}

pub(super) fn position() -> Option<Position> {
    with_pet(|pet| pet.screen_position())
}

pub(super) fn environment() -> super::behavior::Environment {
    with_pet(|pet| {
        let mut environment = super::behavior::Environment::default();
        let scale = SIZE / pet.pixel_size.max(1) as f64;
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
            environment.cursor = Some([
                (cursor.x - pet.origin_x) as f64 * scale,
                (cursor.y - pet.origin_y) as f64 * scale,
            ]);
        }
        environment.pressed = pet.captured;
        environment.dragging = pet.captured && pet.dragged;
        let work = work_area(
            pet.origin_x + pet.pixel_size / 2,
            pet.origin_y + pet.pixel_size / 2,
        );
        environment.floor_distance =
            (work.bottom - pet.pixel_size - pet.origin_y).max(0) as f64 * scale;
        environment.horizontal_room = [
            (pet.origin_x - work.left).max(0) as f64 * scale,
            (work.right - pet.pixel_size - pet.origin_x).max(0) as f64 * scale,
        ];
        environment
    })
    .unwrap_or_default()
}

pub(super) fn move_by(delta: [f64; 2]) -> Result<(), String> {
    if delta == [0.0, 0.0] {
        return Ok(());
    }
    with_pet(|pet| {
        if pet.captured {
            pet.fractional_move = [0.0; 2];
            return;
        }
        let scale = pet.pixel_size as f64 / SIZE;
        let pixels = super::behavior::pixel_motion(delta, scale, &mut pet.fractional_move);
        let (x, y) = clamp_window(
            pet.origin_x + pixels[0],
            pet.origin_y + pixels[1],
            pet.pixel_size,
            pet.origin_x + pet.pixel_size / 2,
            pet.origin_y + pet.pixel_size / 2,
        );
        pet.position_dirty |= x != pet.origin_x || y != pet.origin_y;
        pet.origin_x = x;
        pet.origin_y = y;
    })
    .ok_or_else(|| "Momo window is not open".to_string())
}

pub(super) fn suspended() -> bool {
    with_pet(|pet| pet.is_suspended()).unwrap_or(false)
}

pub(super) fn reduced_motion() -> bool {
    let mut enabled = BOOL(1);
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut BOOL as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && !enabled.as_bool()
    }
}

/// A tracking native tooltip: it does not activate a window or intercept input.
pub(super) fn set_speech(text: Option<&str>) {
    with_pet(|pet| {
        let text = text.filter(|_| !pet.is_suspended());
        let Some(text) = text else {
            if let Some(speech) = &mut pet.speech {
                if speech.visible {
                    let mut info = tool_info(pet.hwnd, speech.wide.as_ptr());
                    unsafe {
                        SendMessageW(
                            speech.hwnd,
                            WM_USER + 17,
                            Some(WPARAM(0)),
                            Some(LPARAM(&mut info as *mut ToolInfo as isize)),
                        );
                    }
                    speech.visible = false;
                }
            }
            return;
        };
        if pet.speech.is_none() {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let hwnd = create_tooltip(pet.hwnd, wide.as_ptr());
            if hwnd.is_invalid() {
                return;
            }
            let mut info = tool_info(pet.hwnd, wide.as_ptr());
            info.flags |= 0x0020 | 0x0080; // TTF_TRACK | TTF_ABSOLUTE
            send_tool(hwnd, WM_USER + 54, &mut info); // TTM_SETTOOLINFOW
            unsafe {
                // Native tooltip colors are ignored while visual styles are active.
                type SetTheme =
                    unsafe extern "system" fn(HWND, PCWSTR, PCWSTR) -> windows::core::HRESULT;
                if let Some(set_theme) =
                    load_symbol::<SetTheme>(w!("uxtheme.dll"), b"SetWindowTheme\0")
                {
                    let _ = set_theme(hwnd, w!(""), w!(""));
                }
                SendMessageW(hwnd, WM_USER + 19, Some(WPARAM(0x302a28)), None);
                SendMessageW(hwnd, WM_USER + 20, Some(WPARAM(0xebebed)), None);
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE,
                    LWA_ALPHA, WS_EX_TRANSPARENT,
                };
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    style | (WS_EX_LAYERED | WS_EX_TRANSPARENT).0 as isize,
                );
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 242, LWA_ALPHA);
                SendMessageW(
                    hwnd,
                    TTM_SETMAXTIPWIDTH,
                    None,
                    Some(LPARAM(scale_px(208.0, pet.dpi) as isize)),
                );
            }
            pet.speech = Some(Speech {
                hwnd,
                text: text.into(),
                wide,
                visible: false,
                origin: (i32::MIN, i32::MIN),
            });
        }
        let speech = pet.speech.as_mut().unwrap();
        let changed = speech.text != text;
        if changed {
            let wide = text.encode_utf16().chain(Some(0)).collect();
            let previous = mem::replace(&mut speech.wide, wide);
            tooltip_text(speech.hwnd, pet.hwnd, speech.wide.as_ptr());
            drop(previous); // The native tooltip now holds the replacement pointer.
            speech.text.clear();
            speech.text.push_str(text);
        }
        if !speech.visible {
            let mut info = tool_info(pet.hwnd, speech.wide.as_ptr());
            unsafe {
                SendMessageW(
                    speech.hwnd,
                    WM_USER + 17,
                    Some(WPARAM(1)),
                    Some(LPARAM(&mut info as *mut ToolInfo as isize)),
                );
            }
        }
        let origin = (pet.origin_x, pet.origin_y);
        if !speech.visible || changed || speech.origin != origin {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetWindowRect(speech.hwnd, &mut rect);
            }
            let work = work_area(pet.origin_x, pet.origin_y);
            let width = (rect.right - rect.left).max(1);
            let height = (rect.bottom - rect.top).max(1);
            let x = (pet.origin_x + scale_px(84.0, pet.dpi) - width)
                .clamp(work.left, (work.right - width).max(work.left));
            let above = pet.origin_y + scale_px(20.0, pet.dpi) - height;
            let y = if above >= work.top {
                above
            } else {
                pet.origin_y + scale_px(88.0, pet.dpi)
            };
            let y = y.clamp(work.top, (work.bottom - height).max(work.top));
            let packed = (x as u16 as u32) | ((y as u16 as u32) << 16);
            unsafe {
                SendMessageW(
                    speech.hwnd,
                    WM_USER + 18,
                    None,
                    Some(LPARAM(packed as isize)),
                );
            }
            speech.origin = origin;
        }
        speech.visible = true;
    });
}

pub(super) fn set_status(text: &str) {
    let prepared = with_pet(|pet| {
        if pet.same_status(text) {
            return None;
        }
        if !pet.write_status(text) {
            eprintln!("Momo status text is too long");
            return None;
        }
        Some((pet.hwnd, pet.tooltip, pet.status.as_ptr()))
    });
    let Some((hwnd, tooltip, text_ptr)) = prepared.flatten() else {
        return;
    };
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        if let Err(error) = SetWindowTextW(hwnd, PCWSTR(text_ptr)) {
            eprintln!("Momo status text was not applied: {error}");
        }
        tooltip_text(tooltip, hwnd, text_ptr);
        NotifyWinEvent(
            EVENT_OBJECT_NAMECHANGE,
            hwnd,
            OBJID_WINDOW.0,
            CHILDID_SELF as i32,
        );
    }
}

fn pet_installed() -> Option<bool> {
    PET.with(|slot| Some(slot.try_borrow().ok()?.is_some()))
}

fn with_pet<T>(body: impl FnOnce(&mut Pet) -> T) -> Option<T> {
    PET.with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        Some(body(slot.as_mut()?))
    })
}

fn reveal() {
    let Some((hwnd, text_ptr)) = with_pet(|pet| (pet.hwnd, pet.status.as_ptr())) else {
        return;
    };
    let tooltip = create_tooltip(hwnd, text_ptr);
    let stored = with_pet(|pet| pet.tooltip = tooltip).is_some();
    if !stored && !tooltip.is_invalid() {
        unsafe {
            let _ = DestroyWindow(tooltip);
        }
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); // Returns previous visibility, not success.
    }
    if register_session(hwnd) {
        let stored = with_pet(|pet| pet.session_notifications = true).is_some();
        if !stored {
            unregister_session(hwnd);
        }
    } else {
        eprintln!("Momo will not observe Windows session lock");
    }
}

fn register_class() -> Result<(), String> {
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    if REGISTERED.load(Ordering::Acquire) {
        return Ok(());
    }
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null())
            .map_err(|error| format!("Momo instance handle: {error}"))?;
        let cursor =
            LoadCursorW(None, IDC_ARROW).map_err(|error| format!("Momo cursor: {error}"))?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hCursor: cursor,
            lpszClassName: w!("Kivio.Momo"),
            ..WNDCLASSEXW::default()
        };
        if RegisterClassExW(&class) == 0 {
            let error = GetLastError();
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(format!(
                    "Register Momo window class: {}",
                    windows::core::Error::from_win32()
                ));
            }
        }
    }
    REGISTERED.store(true, Ordering::Release);
    Ok(())
}

impl Pet {
    fn open(app: &AppHandle, visual: &Visual, position: Option<Position>) -> Result<Self, String> {
        let provisional_dpi = screen_dpi();
        let provisional_size = scale_px(SIZE, provisional_dpi);
        let provisional_origin = choose_origin(position, provisional_size, provisional_dpi);
        let instance = unsafe {
            GetModuleHandleW(PCWSTR::null())
                .map_err(|error| format!("Momo instance handle: {error}"))?
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                w!("Kivio.Momo"),
                w!("Momo · Kivio"),
                WS_POPUP,
                provisional_origin.0,
                provisional_origin.1,
                provisional_size,
                provisional_size,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|error| format!("Create Momo window: {error}"))?
        };
        let mut pet = Self {
            hwnd,
            tooltip: HWND::default(),
            speech: None,
            app: app.clone(),
            visual: *visual,
            origin_x: provisional_origin.0,
            origin_y: provisional_origin.1,
            dpi: provisional_dpi,
            pixel_size: 0,
            surface: Surface::empty(),
            captured: false,
            dragged: false,
            swallow_up: false,
            down_cursor: POINT::default(),
            down_origin: provisional_origin,
            fractional_move: [0.0; 2],
            position_dirty: false,
            power_suspended: false,
            session_locked: false,
            session_notifications: false,
            status: [0; STATUS_CAPACITY],
            status_len: 0,
        };
        let _ = pet.write_status(INITIAL_STATUS);
        let dpi = dpi_of(hwnd);
        pet.dpi = dpi;
        let pixel_size = scale_px(SIZE, dpi);
        let (origin_x, origin_y) = choose_origin(position, pixel_size, dpi);
        pet.origin_x = origin_x;
        pet.origin_y = origin_y;
        pet.rebuild_surface(pixel_size)?;
        pet.blit(visual)?;
        pet.present()?;
        Ok(pet)
    }

    fn redraw(&mut self, visual: &Visual) -> Result<(), String> {
        let dpi = PENDING_DPI.with(|pending| pending.replace(0));
        if dpi != 0 {
            self.dpi = dpi;
        }
        let changed = self.visual != *visual;
        if changed {
            // Hit testing reads this body on the next mouse message. Layered
            // alpha stays with the previous upload until the pixels below change.
            self.visual = *visual;
        }
        let pixel_size = scale_px(SIZE, self.dpi);
        let surface_current = pixel_size == self.pixel_size && !self.surface.bits.is_null();
        if !changed && surface_current {
            // Identical geometry, including a reduced-motion still frame.
            // WM_NCHITTEST and the existing DIB alpha already match the cursor.
            return if mem::take(&mut self.position_dirty) {
                self.present()
            } else {
                Ok(())
            };
        }
        if !surface_current {
            self.rebuild_surface(pixel_size)?;
            let (origin_x, origin_y) = clamp_window(
                self.origin_x,
                self.origin_y,
                pixel_size,
                self.origin_x + pixel_size / 2,
                self.origin_y + pixel_size / 2,
            );
            self.origin_x = origin_x;
            self.origin_y = origin_y;
        }
        self.blit(visual)?;
        self.position_dirty = false;
        self.present()
    }

    fn rebuild_surface(&mut self, pixel_size: i32) -> Result<(), String> {
        let surface = Surface::create(pixel_size)?;
        self.surface = surface;
        self.pixel_size = pixel_size;
        Ok(())
    }

    fn blit(&self, visual: &Visual) -> Result<(), String> {
        let width = self.pixel_size as usize;
        if self.surface.bits.is_null() || width == 0 {
            return Err("Momo window surface is missing".into());
        }
        // The DIB is a top-down 32-bit section: rows are tightly packed and this thread
        // is the only reader or writer until the surface is released.
        let pixels = unsafe { slice::from_raw_parts_mut(self.surface.bits, width * width * 4) };
        visual::rasterize(visual, pixels, width, width);
        Ok(())
    }

    fn present(&self) -> Result<(), String> {
        if self.hwnd.is_invalid() || self.surface.dc.is_invalid() || self.pixel_size <= 0 {
            return Err("Momo window surface is missing".into());
        }
        let destination = POINT {
            x: self.origin_x,
            y: self.origin_y,
        };
        let size = WinSize {
            cx: self.pixel_size,
            cy: self.pixel_size,
        };
        let source = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&destination),
                Some(&size),
                Some(self.surface.dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .map_err(|error| format!("Update Momo window: {error}"))
        }
    }

    fn screen_position(&self) -> Position {
        let mut rect = RECT::default();
        let (x, y) = unsafe {
            if !self.hwnd.is_invalid() && GetWindowRect(self.hwnd, &mut rect).is_ok() {
                (rect.left, rect.top)
            } else {
                (self.origin_x, self.origin_y)
            }
        };
        Position {
            x: x as f64,
            y: y as f64,
        }
    }

    fn is_suspended(&self) -> bool {
        self.power_suspended
            || self.session_locked
            || screensaver_running()
            || self.hwnd.is_invalid()
            || unsafe { !IsWindowVisible(self.hwnd).as_bool() }
    }

    fn same_status(&self, text: &str) -> bool {
        let mut index = 0;
        for unit in text.encode_utf16() {
            if index >= self.status_len || self.status[index] != unit {
                return false;
            }
            index += 1;
        }
        index == self.status_len
    }

    fn write_status(&mut self, text: &str) -> bool {
        let mut index = 0;
        for unit in text.encode_utf16() {
            if index + 1 >= STATUS_CAPACITY {
                return false;
            }
            self.status[index] = unit;
            index += 1;
        }
        self.status[index] = 0;
        self.status_len = index;
        true
    }

    fn hit_test(&self, lparam: LPARAM) -> isize {
        let (x, y) = point_from_lparam(lparam);
        if self.pixel_size <= 0
            || x < self.origin_x
            || y < self.origin_y
            || x >= self.origin_x + self.pixel_size
            || y >= self.origin_y + self.pixel_size
        {
            return HTTRANSPARENT as isize;
        }
        let logical_x = (x - self.origin_x) as f64 * SIZE / self.pixel_size as f64;
        let logical_y = (y - self.origin_y) as f64 * SIZE / self.pixel_size as f64;
        if self.visual.contains(logical_x, logical_y) {
            1 // HTCLIENT
        } else {
            HTTRANSPARENT as isize
        }
    }

    fn begin_drag(&mut self) {
        self.captured = true;
        self.dragged = false;
        self.swallow_up = false;
        self.down_cursor = cursor_pos().unwrap_or(POINT {
            x: self.origin_x,
            y: self.origin_y,
        });
        self.down_origin = (self.origin_x, self.origin_y);
    }

    fn drag_move(&mut self) {
        if !self.captured {
            return;
        }
        let Some(cursor) = cursor_pos() else {
            return;
        };
        let dx = cursor.x - self.down_cursor.x;
        let dy = cursor.y - self.down_cursor.y;
        if !self.dragged {
            let threshold_x = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(1);
            let threshold_y = unsafe { GetSystemMetrics(SM_CYDRAG) }.max(1);
            if dx.abs() < threshold_x && dy.abs() < threshold_y {
                return;
            }
            self.dragged = true;
        }
        let (x, y) = clamp_window(
            self.down_origin.0 + dx,
            self.down_origin.1 + dy,
            self.pixel_size,
            cursor.x,
            cursor.y,
        );
        if x == self.origin_x && y == self.origin_y {
            return;
        }
        self.origin_x = x;
        self.origin_y = y;
        if let Err(error) = self.present() {
            eprintln!("Move Momo window: {error}");
        }
    }

    fn reclamp(&mut self) {
        let (origin_x, origin_y) = clamp_window(
            self.origin_x,
            self.origin_y,
            self.pixel_size,
            self.origin_x + self.pixel_size / 2,
            self.origin_y + self.pixel_size / 2,
        );
        if origin_x == self.origin_x && origin_y == self.origin_y {
            return;
        }
        self.origin_x = origin_x;
        self.origin_y = origin_y;
        if let Err(error) = self.present() {
            eprintln!("Move Momo window: {error}");
        }
    }
}

impl Drop for Pet {
    fn drop(&mut self) {
        self.speech.take();
        if mem::replace(&mut self.session_notifications, false) && !self.hwnd.is_invalid() {
            unregister_session(self.hwnd);
        }
        let tooltip = mem::replace(&mut self.tooltip, HWND::default());
        if !tooltip.is_invalid() {
            unsafe {
                let _ = DestroyWindow(tooltip);
            }
        }
        let hwnd = mem::replace(&mut self.hwnd, HWND::default());
        if !hwnd.is_invalid() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

impl Surface {
    fn empty() -> Self {
        Self {
            dc: HDC::default(),
            bitmap: HBITMAP::default(),
            previous: HGDIOBJ::default(),
            bits: ptr::null_mut(),
        }
    }

    fn create(width: i32) -> Result<Self, String> {
        if width <= 0 {
            return Err("Momo window size is empty".into());
        }
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("Momo screen DC is unavailable".into());
            }
            let dc = CreateCompatibleDC(Some(screen));
            if dc.is_invalid() {
                ReleaseDC(None, screen);
                return Err(last_error("Create Momo memory DC"));
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -width,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: (width as u32)
                        .saturating_mul(width as u32)
                        .saturating_mul(4),
                    ..BITMAPINFOHEADER::default()
                },
                bmiColors: [RGBQUAD::default()],
            };
            let mut bits: *mut c_void = ptr::null_mut();
            let bitmap =
                match CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(bitmap) => bitmap,
                    Err(error) => {
                        if !DeleteDC(dc).as_bool() {
                            eprintln!("Failed to release Momo memory DC");
                        }
                        ReleaseDC(None, screen);
                        return Err(format!("Create Momo bitmap: {error}"));
                    }
                };
            ReleaseDC(None, screen);
            if bits.is_null() {
                if !DeleteObject(HGDIOBJ(bitmap.0)).as_bool() {
                    eprintln!("Failed to release Momo bitmap");
                }
                if !DeleteDC(dc).as_bool() {
                    eprintln!("Failed to release Momo memory DC");
                }
                return Err("Momo bitmap has no pixel buffer".into());
            }
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            if previous.is_invalid() {
                if !DeleteObject(HGDIOBJ(bitmap.0)).as_bool() {
                    eprintln!("Failed to release Momo bitmap");
                }
                if !DeleteDC(dc).as_bool() {
                    eprintln!("Failed to release Momo memory DC");
                }
                return Err(last_error("Select Momo bitmap"));
            }
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
            })
        }
    }

    fn release(&mut self) {
        unsafe {
            if !self.dc.is_invalid() {
                if !self.previous.is_invalid() {
                    SelectObject(self.dc, self.previous);
                    self.previous = HGDIOBJ::default();
                }
                if !DeleteDC(self.dc).as_bool() {
                    eprintln!("Failed to release Momo memory DC");
                }
                self.dc = HDC::default();
            }
            if !self.bitmap.is_invalid() {
                if !DeleteObject(HGDIOBJ(self.bitmap.0)).as_bool() {
                    eprintln!("Failed to release Momo bitmap");
                }
                self.bitmap = HBITMAP::default();
            }
            self.bits = ptr::null_mut();
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.release();
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_CONTEXTMENU => LRESULT(0),
        WM_NCHITTEST => {
            LRESULT(with_pet(|pet| pet.hit_test(lparam)).unwrap_or(HTTRANSPARENT as isize))
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_MOUSEMOVE => {
            let tooltip = with_pet(|pet| pet.tooltip).unwrap_or_default();
            relay_tooltip(tooltip, hwnd, msg, wparam, lparam);
            dispatch_pointer(hwnd, msg);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            // lParam is the window gaining capture. Losing capture to ourselves is
            // SetCapture renewing the drag, not the end of it.
            let gaining = HWND(lparam.0 as *mut c_void);
            if gaining != hwnd {
                dispatch_pointer(hwnd, msg);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xffff) as u32;
            if dpi != 0 {
                PENDING_DPI.with(|pending| pending.set(dpi));
                with_pet(|pet| {
                    let visual = pet.visual;
                    if let Err(error) = pet.redraw(&visual) {
                        eprintln!("Resize Momo window: {error}");
                    }
                });
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            with_pet(|pet| pet.reclamp());
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let kind = wparam.0 as u32;
            with_pet(|pet| {
                if kind == PBT_APMSUSPEND {
                    pet.power_suspended = true;
                } else if kind == PBT_APMRESUMEAUTOMATIC
                    || kind == PBT_APMRESUMESUSPEND
                    || kind == PBT_APMRESUMECRITICAL
                {
                    pet.power_suspended = false;
                }
            });
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            let kind = wparam.0 as u32;
            with_pet(|pet| {
                if kind == WTS_SESSION_LOCK {
                    pet.session_locked = true;
                } else if kind == WTS_SESSION_UNLOCK {
                    pet.session_locked = false;
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            with_pet(|pet| {
                if mem::replace(&mut pet.session_notifications, false) {
                    unregister_session(hwnd);
                }
                pet.tooltip = HWND::default();
                pet.hwnd = HWND::default();
                pet.surface.release();
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn dispatch_pointer(hwnd: HWND, msg: u32) {
    match msg {
        WM_LBUTTONDOWN => {
            with_pet(|pet| pet.begin_drag());
            unsafe {
                SetCapture(hwnd);
            }
        }
        WM_MOUSEMOVE => {
            with_pet(|pet| pet.drag_move());
        }
        WM_LBUTTONUP => {
            let action = with_pet(|pet| {
                let release = pet.captured;
                if !pet.captured && !pet.swallow_up {
                    return (None, false);
                }
                pet.captured = false;
                let dragged = mem::replace(&mut pet.dragged, false);
                if mem::replace(&mut pet.swallow_up, false) {
                    return (None, release);
                }
                let position = Position {
                    x: pet.origin_x as f64,
                    y: pet.origin_y as f64,
                };
                (
                    Some((
                        pet.app.clone(),
                        if dragged {
                            NativeAction::PositionChanged(position)
                        } else {
                            NativeAction::Poke
                        },
                    )),
                    release,
                )
            });
            if let Some((action, release)) = action {
                if release {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                if let Some((app, action)) = action {
                    native_action(&app, action);
                }
            }
        }
        WM_LBUTTONDBLCLK => {
            let action = with_pet(|pet| {
                let release = pet.captured;
                pet.captured = false;
                pet.dragged = false;
                pet.swallow_up = true;
                (pet.app.clone(), release)
            });
            if let Some((app, release)) = action {
                if release {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                native_action(&app, NativeAction::OpenChat);
            }
        }
        WM_RBUTTONUP => {
            let app = with_pet(|pet| {
                if pet.captured {
                    None
                } else {
                    Some(pet.app.clone())
                }
            });
            if let Some(Some(app)) = app {
                native_action(&app, NativeAction::Hide);
            }
        }
        WM_CAPTURECHANGED => {
            let action = with_pet(|pet| {
                if !pet.captured {
                    return None;
                }
                pet.captured = false;
                let dragged = mem::replace(&mut pet.dragged, false);
                if !dragged {
                    return None;
                }
                Some((
                    pet.app.clone(),
                    NativeAction::PositionChanged(Position {
                        x: pet.origin_x as f64,
                        y: pet.origin_y as f64,
                    }),
                ))
            });
            if let Some((app, action)) = action.flatten() {
                native_action(&app, action);
            }
        }
        _ => {}
    }
}

fn choose_origin(position: Option<Position>, size: i32, dpi: u32) -> (i32, i32) {
    if let Some(position) = position {
        if let (Some(x), Some(y)) = (finite_coord(position.x), finite_coord(position.y)) {
            return clamp_window(x, y, size, x, y);
        }
    }
    let cursor = cursor_pos().unwrap_or_default();
    let work = work_area(cursor.x, cursor.y);
    let margin = scale_px(16.0, dpi);
    clamp_window(
        work.right.saturating_sub(size).saturating_sub(margin),
        work.bottom.saturating_sub(size).saturating_sub(margin),
        size,
        cursor.x,
        cursor.y,
    )
}

fn clamp_window(x: i32, y: i32, size: i32, anchor_x: i32, anchor_y: i32) -> (i32, i32) {
    let work = work_area(anchor_x, anchor_y);
    let max_x = work.right.saturating_sub(size).max(work.left);
    let max_y = work.bottom.saturating_sub(size).max(work.top);
    (x.clamp(work.left, max_x), y.clamp(work.top, max_y))
}

fn work_area(x: i32, y: i32) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..MONITORINFO::default()
        };
        if !monitor.is_invalid() && GetMonitorInfoW(monitor, &mut info).as_bool() {
            return info.rcWork;
        }
        let mut rect = RECT::default();
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
        {
            return rect;
        }
        RECT {
            left: 0,
            top: 0,
            right: 128,
            bottom: 128,
        }
    }
}

fn scale_px(logical: f64, dpi: u32) -> i32 {
    let value = (logical * dpi.max(1) as f64 / 96.0).round();
    if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        (value as i32).max(1)
    }
}

fn finite_coord(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    let rounded = value.round();
    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        None
    } else {
        Some(rounded as i32)
    }
}

fn screen_dpi() -> u32 {
    unsafe {
        let dc = GetDC(None);
        let dpi = if dc.is_invalid() {
            0
        } else {
            GetDeviceCaps(Some(dc), LOGPIXELSX)
        };
        if !dc.is_invalid() {
            ReleaseDC(None, dc);
        }
        if dpi > 0 {
            dpi as u32
        } else {
            96
        }
    }
}

fn dpi_of(hwnd: HWND) -> u32 {
    static GET: LazyLock<Option<unsafe extern "system" fn(HWND) -> u32>> =
        LazyLock::new(|| load_symbol(w!("user32.dll"), b"GetDpiForWindow\0"));
    if let Some(getter) = *GET {
        let dpi = unsafe { getter(hwnd) };
        if dpi > 0 {
            return dpi;
        }
    }
    unsafe {
        let dc = GetDC(Some(hwnd));
        let dpi = if dc.is_invalid() {
            0
        } else {
            GetDeviceCaps(Some(dc), LOGPIXELSX)
        };
        if !dc.is_invalid() {
            ReleaseDC(Some(hwnd), dc);
        }
        if dpi > 0 {
            dpi as u32
        } else {
            96
        }
    }
}

fn cursor_pos() -> Option<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok().map(|_| point) }
}

fn point_from_lparam(value: LPARAM) -> (i32, i32) {
    let packed = value.0 as u32;
    let x = (packed & 0xffff) as i16 as i32;
    let y = ((packed >> 16) & 0xffff) as i16 as i32;
    (x, y)
}

fn screensaver_running() -> bool {
    let mut running = BOOL(0);
    unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENSAVERRUNNING,
            0,
            Some(&mut running as *mut BOOL as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && running.as_bool()
    }
}

fn last_error(context: &str) -> String {
    format!("{context}: {}", windows::core::Error::from_win32())
}

#[derive(Clone, Copy)]
struct SessionApi {
    register: unsafe extern "system" fn(HWND, u32) -> i32,
    unregister: unsafe extern "system" fn(HWND) -> i32,
}

fn session_api() -> Option<SessionApi> {
    static API: LazyLock<Option<SessionApi>> = LazyLock::new(|| {
        let register = load_symbol(w!("wtsapi32.dll"), b"WTSRegisterSessionNotification\0")?;
        let unregister = load_symbol(w!("wtsapi32.dll"), b"WTSUnRegisterSessionNotification\0")?;
        Some(SessionApi {
            register,
            unregister,
        })
    });
    *API
}

fn register_session(hwnd: HWND) -> bool {
    let Some(api) = session_api() else {
        return false;
    };
    unsafe { (api.register)(hwnd, NOTIFY_FOR_THIS_SESSION) != 0 }
}

fn unregister_session(hwnd: HWND) {
    let Some(api) = session_api() else {
        return;
    };
    unsafe {
        let _ = (api.unregister)(hwnd);
    }
}

fn load_symbol<T>(library: PCWSTR, name: &'static [u8]) -> Option<T> {
    unsafe {
        if size_of::<T>() != size_of::<unsafe extern "system" fn() -> isize>() {
            return None;
        }
        // The library remains loaded so the returned function pointer stays valid.
        let module = LoadLibraryW(library).ok()?;
        let symbol = GetProcAddress(module, PCSTR::from_raw(name.as_ptr()))?;
        Some(mem::transmute_copy(&symbol))
    }
}

#[repr(C)]
struct ToolInfo {
    cb_size: u32,
    flags: u32,
    hwnd: HWND,
    id: usize,
    rect: RECT,
    instance: HINSTANCE,
    text: *mut u16,
    param: isize,
    reserved: *mut c_void,
}

fn create_tooltip(owner: HWND, text: *const u16) -> HWND {
    static CONTROLS_READY: LazyLock<bool> = LazyLock::new(|| unsafe {
        if LoadLibraryW(w!("comctl32.dll")).is_err() {
            return false;
        }
        init_common_controls();
        true
    });
    if !*CONTROLS_READY {
        return HWND::default();
    }
    unsafe {
        let tip = open_tooltip(owner);
        if tip.is_invalid() || !install_tooltip(tip, owner, text) {
            if !tip.is_invalid() {
                let _ = DestroyWindow(tip);
            }
            HWND::default()
        } else {
            tip
        }
    }
}

fn open_tooltip(owner: HWND) -> HWND {
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()).unwrap_or_default() };
    unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("tooltips_class32"),
            PCWSTR::null(),
            WINDOW_STYLE(WS_POPUP.0 | TOOLTIP_STYLE),
            0,
            0,
            0,
            0,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap_or_default()
    }
}

fn install_tooltip(tip: HWND, owner: HWND, text: *const u16) -> bool {
    let mut info = tool_info(owner, text);
    unsafe {
        let mut added = send_tool(tip, TTM_ADDTOOLW, &mut info);
        if added == 0 {
            // Common-controls v5 reads the structure without the trailing reserved pointer.
            info.cb_size = info.cb_size.saturating_sub(size_of::<*mut c_void>() as u32);
            added = send_tool(tip, TTM_ADDTOOLW, &mut info);
        }
        if added == 0 {
            return false;
        }
        SendMessageW(tip, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(480)));
    }
    true
}

fn close_tooltip(pet: &mut Pet) {
    pet.speech.take();
    let tooltip = mem::replace(&mut pet.tooltip, HWND::default());
    if !tooltip.is_invalid() {
        unsafe {
            let _ = DestroyWindow(tooltip);
        }
    }
}

fn tooltip_text(tip: HWND, owner: HWND, text: *const u16) {
    if tip.is_invalid() {
        return;
    }
    let mut info = tool_info(owner, text);
    if send_tool(tip, TTM_UPDATETIPTEXTW, &mut info) == 0 {
        info.cb_size = info.cb_size.saturating_sub(size_of::<*mut c_void>() as u32);
        send_tool(tip, TTM_UPDATETIPTEXTW, &mut info);
    }
}

fn send_tool(tip: HWND, message: u32, info: &mut ToolInfo) -> isize {
    unsafe {
        SendMessageW(
            tip,
            message,
            None,
            Some(LPARAM(info as *mut ToolInfo as isize)),
        )
        .0
    }
}

fn tool_info(owner: HWND, text: *const u16) -> ToolInfo {
    ToolInfo {
        cb_size: size_of::<ToolInfo>() as u32,
        flags: TOOLTIP_FLAGS,
        hwnd: owner,
        id: owner.0 as usize,
        rect: RECT::default(),
        instance: HINSTANCE::default(),
        text: text as *mut u16,
        param: 0,
        reserved: ptr::null_mut(),
    }
}

fn relay_tooltip(tip: HWND, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    if tip.is_invalid() {
        return;
    }
    let packed = unsafe { GetMessagePos() };
    let mut message = MSG {
        hwnd,
        message: msg,
        wParam: wparam,
        lParam: lparam,
        time: unsafe { GetMessageTime() } as u32,
        pt: POINT {
            x: (packed & 0xffff) as i16 as i32,
            y: ((packed >> 16) & 0xffff) as i16 as i32,
        },
    };
    unsafe {
        SendMessageW(
            tip,
            TTM_RELAYEVENT,
            None,
            Some(LPARAM(&mut message as *mut MSG as isize)),
        );
    }
}

fn init_common_controls() {
    #[repr(C)]
    struct InitCommonControlsEx {
        size: u32,
        icc: u32,
    }
    static INIT: LazyLock<Option<unsafe extern "system" fn(*const InitCommonControlsEx) -> i32>> =
        LazyLock::new(|| load_symbol(w!("comctl32.dll"), b"InitCommonControlsEx\0"));
    if let Some(init) = *INIT {
        let data = InitCommonControlsEx {
            size: size_of::<InitCommonControlsEx>() as u32,
            icc: 0x0000_40FF,
        };
        unsafe {
            let _ = init(&data);
        }
    }
}
