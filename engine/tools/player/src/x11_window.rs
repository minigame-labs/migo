//! Minimal X11 window for the dev player.
//!
//! The player is the one component allowed to own a toplevel window: the SDK
//! renders into a host-provided surface and never creates a window itself,
//! which `scripts/test-surface-attachment-contract.sh` enforces. Here the player
//! *is* the host: it opens the display, creates the window, and hands the two
//! opaque handles to `LinuxX11Context`.
//!
//! Xlib is loaded at runtime through `x11-dl`, so neither the player nor the
//! SDK gains a link-time X11 dependency — the same posture the desktop
//! presenter takes with the system EGL runtime.
//!
//! `winit` would be the obvious alternative and §7.2 even names it, but the
//! window-handle traits it exposes are exactly the ones
//! `scripts/test-surface-attachment-contract.sh` forbids anywhere under
//! `engine/crates` (it greps for the symbol names, so even naming them here
//! would fail the gate). The enforced gate wins over the unimplemented
//! intention.

use std::{
    ffi::{CString, c_ulong, c_void},
    ptr::NonNull,
};

use x11_dl::xlib;

/// A mapped X11 window plus the connection it belongs to.
///
/// Owns both handles and closes them on drop, so it must outlive the engine
/// session that renders into it.
pub struct X11Window {
    xlib: xlib::Xlib,
    display: NonNull<xlib::Display>,
    window: c_ulong,
    wm_delete_window: xlib::Atom,
    width: u32,
    height: u32,
    close_requested: bool,
}

impl X11Window {
    /// Open a display, create a window of `width` x `height`, and map it.
    pub fn open(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let xlib = xlib::Xlib::open().map_err(|error| format!("load Xlib: {error}"))?;

        // SAFETY: every call below is a plain Xlib call with the arguments its
        // manual page specifies, and the display pointer is checked for null
        // before any use. Migo opens a separate render connection.
        unsafe {
            let display = NonNull::new((xlib.XOpenDisplay)(std::ptr::null())).ok_or_else(|| {
                let target = std::env::var("DISPLAY").unwrap_or_else(|_| "(DISPLAY unset)".into());
                format!("XOpenDisplay failed for {target}")
            })?;
            let display_ptr = display.as_ptr();

            let screen = (xlib.XDefaultScreen)(display_ptr);
            let root = (xlib.XRootWindow)(display_ptr, screen);
            let black = (xlib.XBlackPixel)(display_ptr, screen);
            let window =
                (xlib.XCreateSimpleWindow)(display_ptr, root, 0, 0, width, height, 0, black, black);

            if let Ok(title) = CString::new(title) {
                (xlib.XStoreName)(display_ptr, window, title.as_ptr());
            }
            (xlib.XSelectInput)(display_ptr, window, xlib::StructureNotifyMask);

            // Ask the window manager to route the close button to us instead of
            // killing the connection underneath the render thread.
            let mut wm_delete_window = intern_atom(&xlib, display_ptr, "WM_DELETE_WINDOW")?;
            (xlib.XSetWMProtocols)(display_ptr, window, &mut wm_delete_window, 1);

            (xlib.XMapWindow)(display_ptr, window);
            (xlib.XFlush)(display_ptr);

            Ok(Self {
                xlib,
                display,
                window,
                wm_delete_window,
                width,
                height,
                close_requested: false,
            })
        }
    }

    /// The X11 `Display*`, borrowed synchronously by `LinuxX11Context::open`.
    pub fn display(&self) -> NonNull<c_void> {
        self.display.cast()
    }

    /// The host-owned window XID.
    pub fn window(&self) -> c_ulong {
        self.window
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Drain pending X events. Returns `false` once the window manager has
    /// asked the window to close.
    ///
    /// Resize is deliberately not handled: growing the window would have to
    /// travel through the engine's surface generation/lease path, which is its
    /// own slice (see the design doc). The window is fixed-size until then.
    pub fn pump(&mut self) -> bool {
        // SAFETY: `XPending`/`XNextEvent` operate on the connection this type
        // owns; the event union is fully initialised by `XNextEvent` before we
        // read it.
        unsafe {
            let display = self.display.as_ptr();
            while (self.xlib.XPending)(display) > 0 {
                let mut event: xlib::XEvent = std::mem::zeroed();
                (self.xlib.XNextEvent)(display, &mut event);
                if event.get_type() == xlib::ClientMessage {
                    let message = event.client_message;
                    if message.data.get_long(0) as xlib::Atom == self.wm_delete_window {
                        self.close_requested = true;
                    }
                }
            }
        }
        !self.close_requested
    }
}

impl Drop for X11Window {
    fn drop(&mut self) {
        // SAFETY: both handles were produced by the calls above and are
        // released exactly once, here.
        unsafe {
            let display = self.display.as_ptr();
            (self.xlib.XDestroyWindow)(display, self.window);
            (self.xlib.XCloseDisplay)(display);
        }
    }
}

/// # Safety
/// `display` must be a live connection owned by the caller.
unsafe fn intern_atom(
    xlib: &xlib::Xlib,
    display: *mut xlib::Display,
    name: &str,
) -> Result<xlib::Atom, String> {
    let name = CString::new(name).map_err(|error| format!("atom name: {error}"))?;
    Ok(unsafe { (xlib.XInternAtom)(display, name.as_ptr(), xlib::False) })
}
