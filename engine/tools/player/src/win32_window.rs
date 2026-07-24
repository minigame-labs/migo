//! A real Win32 window for the player's onscreen mode.
//!
//! The mirror of `x11_window.rs`: the player is the *host*, so it owns the
//! window and hands the engine nothing but an opaque `HWND`. That is the same
//! split the SDK enforces everywhere else -- `WindowsHwndSurface` takes a
//! handle and never learns what created it, so nothing in the engine links
//! user32.
//!
//! The Win32 calls are declared here rather than pulled from a bindings crate on
//! purpose. `engine/Cargo.lock` is part of the V8 snapshot fingerprint, so a new
//! dependency would mark all six committed snapshots stale and cost a
//! regeneration run on real hardware for each ABI. Twelve `extern "system"`
//! declarations are cheaper than that, and they are the stable Win32 ABI.

use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{NonNull, null, null_mut};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Wparam = usize;
type Lparam = isize;
type Lresult = isize;

#[repr(C)]
struct WndClassW {
    style: u32,
    lpfn_wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: Hinstance,
    h_icon: *mut c_void,
    h_cursor: *mut c_void,
    hbr_background: *mut c_void,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
    time: u32,
    pt: Point,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, w: Wparam, l: Lparam) -> Lresult;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    fn UpdateWindow(hwnd: Hwnd) -> i32;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> Lresult;
    fn PostQuitMessage(code: i32);
    fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn AdjustWindowRect(rect: *mut Rect, style: u32, menu: i32) -> i32;
}

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const SW_SHOW: i32 = 5;
const PM_REMOVE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const CW_USEDEFAULT: i32 = i32::MIN;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, w: Wparam, l: Lparam) -> Lresult {
    match msg {
        // Only ask the loop to stop; the window itself is destroyed in `Drop`,
        // after the render thread has let go of the EGL surface built from it.
        WM_CLOSE => {
            unsafe { PostQuitMessage(0) };
            0
        }
        WM_DESTROY => 0,
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

/// A shown, host-owned window. Dropping it destroys the native window, so it
/// must outlive the engine session that renders into it.
pub struct Win32Window {
    hwnd: Hwnd,
    width: u32,
    height: u32,
    closing: bool,
}

impl Win32Window {
    pub fn open(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let class_name = wide("MigoPlayerWindow");
        let title_w = wide(title);

        // SAFETY: every pointer below is either null or a live NUL-terminated
        // wide string owned by this function for the duration of the calls.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = WndClassW {
                style: 0,
                lpfn_wnd_proc: Some(wnd_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: instance,
                h_icon: null_mut(),
                h_cursor: null_mut(),
                hbr_background: null_mut(),
                lpsz_menu_name: null(),
                lpsz_class_name: class_name.as_ptr(),
            };
            // A zero return can also mean "already registered" from an earlier
            // open in the same process, which is not an error here.
            RegisterClassW(&class);

            // Size the frame so the *client* area is the requested size: the
            // engine renders into the client area, and asking for the outer
            // size would hand it a surface a title bar shorter than the game
            // expects.
            let mut rect = Rect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW returned null".to_string());
            }
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);

            // Report the client size the window actually got, not the size that
            // was asked for: the engine builds its surface from this, and a
            // mismatch shows up as content drawn at the wrong scale.
            let mut client = Rect::default();
            GetClientRect(hwnd, &mut client);
            let (w, h) = (
                (client.right - client.left).max(1) as u32,
                (client.bottom - client.top).max(1) as u32,
            );

            Ok(Self {
                hwnd,
                width: w,
                height: h,
                closing: false,
            })
        }
    }

    /// The `HWND`, for `WindowsHwndSurface`.
    pub fn hwnd(&self) -> NonNull<c_void> {
        NonNull::new(self.hwnd).expect("window handle checked non-null at open")
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Drain pending messages. Returns false once the window has been asked to
    /// close, so the caller still runs its capture and shutdown path instead of
    /// being killed mid-frame.
    pub fn pump(&mut self) -> bool {
        let mut msg = Msg {
            hwnd: null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: Point { x: 0, y: 0 },
        };
        // SAFETY: `msg` is a live, correctly sized message struct.
        unsafe {
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    self.closing = true;
                    return false;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        !self.closing
    }
}

impl Drop for Win32Window {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: the handle came from CreateWindowExW and has not been
            // destroyed; the caller drops this only after the engine reports the
            // surface released.
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = null_mut();
        }
    }
}
