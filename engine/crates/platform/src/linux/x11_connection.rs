use std::{
    ffi::{CStr, CString, c_char, c_void},
    fmt,
    os::fd::RawFd,
    ptr::NonNull,
    sync::Arc,
};

use shared::error::{EngineError, EngineResult, ErrorCode};

const LINUX_X11_LIBRARY: &str = "libX11.so.6";

type XDisplayString = unsafe extern "C" fn(*mut c_void) -> *mut c_char;
type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type XCloseDisplay = unsafe extern "C" fn(*mut c_void) -> libc::c_int;
type XConnectionNumber = unsafe extern "C" fn(*mut c_void) -> libc::c_int;

#[derive(Clone, Debug, PartialEq, Eq)]
enum X11ServerIdentity {
    Unix {
        pid: libc::pid_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
        display_number: Box<[u8]>,
    },
    Inet {
        address: [u8; 4],
        port: u16,
        display_number: Box<[u8]>,
    },
    Inet6 {
        address: [u8; 16],
        port: u16,
        scope_id: u32,
        display_number: Box<[u8]>,
    },
}

trait X11Api: fmt::Debug + Send + Sync {
    fn display_name(&self, display: NonNull<c_void>) -> EngineResult<CString>;
    fn open_display(&self, name: &CStr) -> EngineResult<NonNull<c_void>>;
    fn close_display(&self, display: NonNull<c_void>);
    fn connection_fd(&self, display: NonNull<c_void>) -> EngineResult<RawFd>;
    fn server_identity(
        &self,
        display: NonNull<c_void>,
        display_name: &CStr,
    ) -> EngineResult<X11ServerIdentity>;
    fn connection_is_alive(&self, fd: RawFd) -> EngineResult<()>;
}

struct DynamicX11Api {
    _library: libloading::Library,
    display_string: XDisplayString,
    open_display: XOpenDisplay,
    close_display: XCloseDisplay,
    connection_number: XConnectionNumber,
}

impl fmt::Debug for DynamicX11Api {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DynamicX11Api")
            .field("library", &LINUX_X11_LIBRARY)
            .finish_non_exhaustive()
    }
}

impl DynamicX11Api {
    fn load() -> EngineResult<Self> {
        let library = unsafe { libloading::Library::new(LINUX_X11_LIBRARY) }.map_err(|error| {
            EngineError::new(ErrorCode::RenderBackendError)
                .with_msg("load X11 client library failed")
                .with_detail(format!("{LINUX_X11_LIBRARY}: {error}"))
        })?;

        unsafe fn symbol<T: Copy>(
            library: &libloading::Library,
            name: &'static [u8],
        ) -> EngineResult<T> {
            let value = unsafe { library.get::<T>(name) }.map_err(|error| {
                EngineError::new(ErrorCode::RenderBackendError)
                    .with_msg("resolve required X11 symbol failed")
                    .with_detail(format!(
                        "{}: {error}",
                        String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
                    ))
            })?;
            Ok(*value)
        }

        let display_string = unsafe { symbol(&library, b"XDisplayString\0") }?;
        let open_display = unsafe { symbol(&library, b"XOpenDisplay\0") }?;
        let close_display = unsafe { symbol(&library, b"XCloseDisplay\0") }?;
        let connection_number = unsafe { symbol(&library, b"XConnectionNumber\0") }?;
        Ok(Self {
            _library: library,
            display_string,
            open_display,
            close_display,
            connection_number,
        })
    }
}

impl X11Api for DynamicX11Api {
    fn display_name(&self, display: NonNull<c_void>) -> EngineResult<CString> {
        let name =
            NonNull::new(unsafe { (self.display_string)(display.as_ptr()) }).ok_or_else(|| {
                EngineError::new(ErrorCode::InvalidArgument)
                    .with_msg("XDisplayString returned null for the host display")
            })?;
        let bytes = unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes();
        CString::new(bytes).map_err(|_| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("XDisplayString returned an invalid display name")
        })
    }

    fn open_display(&self, name: &CStr) -> EngineResult<NonNull<c_void>> {
        NonNull::new(unsafe { (self.open_display)(name.as_ptr()) }).ok_or_else(|| {
            EngineError::new(ErrorCode::RenderInitializeError)
                .with_msg("open owned X11 render connection failed")
                .with_detail(name.to_string_lossy())
        })
    }

    fn close_display(&self, display: NonNull<c_void>) {
        let _ = unsafe { (self.close_display)(display.as_ptr()) };
    }

    fn connection_fd(&self, display: NonNull<c_void>) -> EngineResult<RawFd> {
        let fd = unsafe { (self.connection_number)(display.as_ptr()) };
        if fd < 0 {
            Err(EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("XConnectionNumber returned an invalid file descriptor"))
        } else {
            Ok(fd)
        }
    }

    fn server_identity(
        &self,
        display: NonNull<c_void>,
        display_name: &CStr,
    ) -> EngineResult<X11ServerIdentity> {
        server_identity_from_fd(self.connection_fd(display)?, display_name)
    }

    fn connection_is_alive(&self, fd: RawFd) -> EngineResult<()> {
        connection_is_alive(fd)
    }
}

struct PendingDisplay {
    api: Option<Arc<dyn X11Api>>,
    display: NonNull<c_void>,
}

impl PendingDisplay {
    fn commit(
        mut self,
        display_name: CString,
        server_identity: X11ServerIdentity,
        fd: RawFd,
    ) -> X11RenderConnection {
        let api = self.api.take().expect("pending X11 API owner must exist");
        X11RenderConnection {
            api,
            display: self.display,
            display_name,
            server_identity,
            fd,
        }
    }
}

impl Drop for PendingDisplay {
    fn drop(&mut self) {
        if let Some(api) = self.api.take() {
            api.close_display(self.display);
        }
    }
}

pub(super) struct X11RenderConnection {
    api: Arc<dyn X11Api>,
    display: NonNull<c_void>,
    display_name: CString,
    server_identity: X11ServerIdentity,
    fd: RawFd,
}

impl fmt::Debug for X11RenderConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("X11RenderConnection")
            .field("display", &self.display)
            .field("display_name", &self.display_name)
            .field("server_identity", &self.server_identity)
            .field("fd", &self.fd)
            .finish()
    }
}

// SAFETY: this pointer is Migo's private X11 connection. Construction and
// compatibility checks finish before a Host starts. Afterwards only EGL on the
// render thread receives the Display pointer; the X11 provider's mandatory
// RenderThreadOnly policy prevents the shared-context upload worker from using
// the same native connection concurrently. Reuse checks poll the copied fd and
// touch only the caller's separate Display.
unsafe impl Send for X11RenderConnection {}
unsafe impl Sync for X11RenderConnection {}

impl X11RenderConnection {
    /// Open a private render connection to the server behind `host_display`.
    ///
    /// # Safety
    /// `host_display` must be a live Xlib `Display*` for this entire call.
    pub(super) unsafe fn open(host_display: NonNull<c_void>) -> EngineResult<Arc<Self>> {
        let api: Arc<dyn X11Api> = Arc::new(DynamicX11Api::load()?);
        Self::open_with_api(host_display, api)
    }

    fn open_with_api(
        host_display: NonNull<c_void>,
        api: Arc<dyn X11Api>,
    ) -> EngineResult<Arc<Self>> {
        let display_name = api.display_name(host_display)?;
        let host_identity = api.server_identity(host_display, &display_name)?;
        let owned_display = api.open_display(&display_name)?;
        let pending = PendingDisplay {
            api: Some(Arc::clone(&api)),
            display: owned_display,
        };
        let owned_identity = api.server_identity(owned_display, &display_name)?;
        if owned_identity != host_identity {
            return Err(EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("owned render connection reached a different X11 server")
                .with_detail(display_name.to_string_lossy()));
        }
        let fd = api.connection_fd(owned_display)?;
        Ok(Arc::new(pending.commit(display_name, owned_identity, fd)))
    }

    #[inline]
    pub(super) fn display(&self) -> NonNull<c_void> {
        self.display
    }

    #[cfg(test)]
    pub(super) fn fd_for_test(&self) -> RawFd {
        self.fd
    }

    /// Verify a later caller-owned display reaches this live render server.
    ///
    /// # Safety
    /// `host_display` must be a live Xlib `Display*` for this entire call.
    pub(super) unsafe fn supports_host_display(
        &self,
        host_display: NonNull<c_void>,
    ) -> EngineResult<()> {
        self.api.connection_is_alive(self.fd)?;
        let display_name = self.api.display_name(host_display)?;
        let candidate = self.api.server_identity(host_display, &display_name)?;
        if candidate == self.server_identity {
            Ok(())
        } else {
            Err(EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("X11 window belongs to a different server")
                .with_detail(display_name.to_string_lossy()))
        }
    }
}

impl Drop for X11RenderConnection {
    fn drop(&mut self) {
        self.api.close_display(self.display);
    }
}

fn display_number(display_name: &CStr) -> EngineResult<Box<[u8]>> {
    let bytes = display_name.to_bytes();
    let separator = bytes
        .iter()
        .rposition(|byte| *byte == b':')
        .ok_or_else(|| {
            EngineError::new(ErrorCode::InvalidArgument)
                .with_msg("X11 display name has no display-number separator")
                .with_detail(display_name.to_string_lossy())
        })?;
    let suffix = &bytes[separator + 1..];
    let number_end = suffix
        .iter()
        .position(|byte| *byte == b'.')
        .unwrap_or(suffix.len());
    let number = &suffix[..number_end];
    if number.is_empty() || !number.iter().all(u8::is_ascii_digit) {
        return Err(EngineError::new(ErrorCode::InvalidArgument)
            .with_msg("X11 display name has an invalid display number")
            .with_detail(display_name.to_string_lossy()));
    }
    Ok(number.into())
}

fn server_identity_from_fd(fd: RawFd, display_name: &CStr) -> EngineResult<X11ServerIdentity> {
    let mut address = std::mem::MaybeUninit::<libc::sockaddr_storage>::zeroed();
    let mut length = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    if unsafe {
        libc::getpeername(
            fd,
            address.as_mut_ptr().cast::<libc::sockaddr>(),
            &mut length,
        )
    } != 0
    {
        return Err(os_error(
            ErrorCode::InvalidArgument,
            "inspect X11 connection peer failed",
        ));
    }
    let address = unsafe { address.assume_init() };
    let display_number = display_number(display_name)?;
    match address.ss_family as libc::c_int {
        libc::AF_UNIX => {
            let mut credentials = std::mem::MaybeUninit::<libc::ucred>::zeroed();
            let mut credentials_length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    credentials.as_mut_ptr().cast::<c_void>(),
                    &mut credentials_length,
                )
            } != 0
            {
                return Err(os_error(
                    ErrorCode::InvalidArgument,
                    "inspect X11 Unix peer credentials failed",
                ));
            }
            if credentials_length as usize != std::mem::size_of::<libc::ucred>() {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_msg("X11 Unix peer returned malformed credentials"));
            }
            let credentials = unsafe { credentials.assume_init() };
            Ok(X11ServerIdentity::Unix {
                pid: credentials.pid,
                uid: credentials.uid,
                gid: credentials.gid,
                display_number,
            })
        }
        libc::AF_INET => {
            if (length as usize) < std::mem::size_of::<libc::sockaddr_in>() {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_msg("X11 IPv4 peer address is truncated"));
            }
            let address: libc::sockaddr_in =
                unsafe { std::ptr::read((&address as *const libc::sockaddr_storage).cast()) };
            Ok(X11ServerIdentity::Inet {
                address: address.sin_addr.s_addr.to_ne_bytes(),
                port: u16::from_be(address.sin_port),
                display_number,
            })
        }
        libc::AF_INET6 => {
            if (length as usize) < std::mem::size_of::<libc::sockaddr_in6>() {
                return Err(EngineError::new(ErrorCode::InvalidArgument)
                    .with_msg("X11 IPv6 peer address is truncated"));
            }
            let address: libc::sockaddr_in6 =
                unsafe { std::ptr::read((&address as *const libc::sockaddr_storage).cast()) };
            Ok(X11ServerIdentity::Inet6 {
                address: address.sin6_addr.s6_addr,
                port: u16::from_be(address.sin6_port),
                scope_id: address.sin6_scope_id,
                display_number,
            })
        }
        family => Err(EngineError::new(ErrorCode::Unsupported)
            .with_msg("X11 connection uses an unsupported transport")
            .with_detail(format!("socket_family={family}"))),
    }
}

fn connection_is_alive(fd: RawFd) -> EngineResult<()> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLRDHUP,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if result >= 0 {
            break;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("poll owned X11 render connection failed")
                .with_detail(error.to_string()));
        }
    }

    let terminal = libc::POLLERR | libc::POLLHUP | libc::POLLNVAL | libc::POLLRDHUP;
    if descriptor.revents & terminal == 0 {
        Ok(())
    } else {
        Err(EngineError::new(ErrorCode::InvalidOperation)
            .with_msg("owned X11 render connection is no longer live")
            .with_detail(format!("poll_revents=0x{:x}", descriptor.revents)))
    }
}

fn os_error(code: ErrorCode, message: &'static str) -> EngineError {
    EngineError::new(code)
        .with_msg(message)
        .with_detail(std::io::Error::last_os_error().to_string())
}

/// A declared X11 topology with no Xlib, no server and no socket: each
/// `Display*` the caller places reaches a named server, and a render connection
/// is opened through the production path against it.
///
/// This is the only way to get a `LinuxX11Context` without a running X server,
/// so both this crate's presenter tests and the C ABI layer's reattachment
/// tests use it. Two ways to fabricate the same object would be two chances to
/// fabricate one production code never builds.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct X11TestServers {
    /// `Display*` as an address, and the X server behind it. Addresses rather
    /// than pointers so the topology stays `Send + Sync` without an unsafe
    /// promise about pointers nothing here dereferences.
    servers: std::collections::HashMap<usize, u8>,
    opens: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(any(test, feature = "test-support"))]
impl X11TestServers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `display` reaches X server `server_id`. A display that was
    /// never placed is unknown to this topology and fails the way Xlib fails on
    /// a pointer from somewhere else.
    pub fn place(&mut self, display: NonNull<c_void>, server_id: u8) -> &mut Self {
        self.servers.insert(display.as_ptr() as usize, server_id);
        self
    }

    /// Private render connections opened through this topology so far.
    ///
    /// The count is what makes "reattachment reuses the connection" falsifiable:
    /// a reuse arm that quietly opened a second one would still return a working
    /// context.
    pub fn opens(&self) -> usize {
        self.opens.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Open a private render connection on `render`, resolved from `host`
    /// exactly as `X11RenderConnection::open` resolves it from a live display.
    pub(super) fn open_connection(
        &self,
        host: NonNull<c_void>,
        render: NonNull<c_void>,
    ) -> EngineResult<Arc<X11RenderConnection>> {
        let api = Arc::new(TestX11Api {
            servers: self.servers.clone(),
            render: render.as_ptr() as usize,
            opens: Arc::clone(&self.opens),
        });
        X11RenderConnection::open_with_api(host, api)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
struct TestX11Api {
    servers: std::collections::HashMap<usize, u8>,
    render: usize,
    opens: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestX11Api {
    fn server_of(&self, display: NonNull<c_void>) -> EngineResult<u8> {
        self.servers
            .get(&(display.as_ptr() as usize))
            .copied()
            .ok_or_else(|| {
                EngineError::new(ErrorCode::InvalidArgument)
                    .with_msg("display was never placed on a test X11 server")
                    .with_detail(format!("display={:p}", display.as_ptr()))
            })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl X11Api for TestX11Api {
    fn display_name(&self, display: NonNull<c_void>) -> EngineResult<CString> {
        let server = self.server_of(display)?;
        CString::new(format!(":{server}")).map_err(|_| {
            EngineError::new(ErrorCode::InvalidArgument).with_msg("test display name contains NUL")
        })
    }

    fn open_display(&self, _name: &CStr) -> EngineResult<NonNull<c_void>> {
        self.opens
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        NonNull::new(self.render as *mut c_void).ok_or_else(|| {
            EngineError::new(ErrorCode::RenderInitializeError)
                .with_msg("test render display must be non-null")
        })
    }

    fn close_display(&self, _display: NonNull<c_void>) {}

    fn connection_fd(&self, display: NonNull<c_void>) -> EngineResult<RawFd> {
        self.server_of(display).map(RawFd::from)
    }

    fn server_identity(
        &self,
        display: NonNull<c_void>,
        _display_name: &CStr,
    ) -> EngineResult<X11ServerIdentity> {
        let server = self.server_of(display)?;
        Ok(X11ServerIdentity::Unix {
            pid: libc::pid_t::from(server),
            uid: 1000,
            gid: 1000,
            display_number: server.to_string().into_bytes().into_boxed_slice(),
        })
    }

    fn connection_is_alive(&self, _fd: RawFd) -> EngineResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        ffi::{CStr, CString, c_void},
        os::fd::RawFd,
        ptr::NonNull,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use parking_lot::Mutex;
    use shared::error::{EngineError, EngineResult, ErrorCode};

    use crate::linux::presenter::LinuxX11Context;

    use super::{DynamicX11Api, X11Api, X11RenderConnection, X11ServerIdentity};

    fn pointer(value: usize) -> NonNull<c_void> {
        NonNull::new(value as *mut c_void).expect("test pointer is non-null")
    }

    fn server(number: u8) -> X11ServerIdentity {
        X11ServerIdentity::Unix {
            pid: number as i32,
            uid: 1000,
            gid: 1000,
            display_number: vec![b'0' + number].into_boxed_slice(),
        }
    }

    #[derive(Debug)]
    struct FakeX11Api {
        name: CString,
        opened: Option<NonNull<c_void>>,
        identities: HashMap<usize, X11ServerIdentity>,
        alive: AtomicBool,
        opens: AtomicUsize,
        closed: Mutex<Vec<usize>>,
        opened_names: Mutex<Vec<Vec<u8>>>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    unsafe impl Send for FakeX11Api {}
    unsafe impl Sync for FakeX11Api {}

    impl FakeX11Api {
        fn matching(host: NonNull<c_void>, owned: NonNull<c_void>) -> Self {
            Self {
                name: CString::new(":7.0").unwrap(),
                opened: Some(owned),
                identities: HashMap::from([
                    (host.as_ptr() as usize, server(7)),
                    (owned.as_ptr() as usize, server(7)),
                ]),
                alive: AtomicBool::new(true),
                opens: AtomicUsize::new(0),
                closed: Mutex::new(Vec::new()),
                opened_names: Mutex::new(Vec::new()),
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Drop for FakeX11Api {
        fn drop(&mut self) {
            self.events.lock().push("api-drop");
        }
    }

    impl X11Api for FakeX11Api {
        fn display_name(&self, _display: NonNull<c_void>) -> EngineResult<CString> {
            Ok(self.name.clone())
        }

        fn open_display(&self, name: &CStr) -> EngineResult<NonNull<c_void>> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            self.opened_names.lock().push(name.to_bytes().to_vec());
            self.opened.ok_or_else(|| {
                EngineError::new(ErrorCode::RenderInitializeError)
                    .with_msg("open owned X11 render connection failed")
                    .with_detail(name.to_string_lossy())
            })
        }

        fn close_display(&self, display: NonNull<c_void>) {
            self.closed.lock().push(display.as_ptr() as usize);
            self.events.lock().push("close-owned");
        }

        fn connection_fd(&self, display: NonNull<c_void>) -> EngineResult<RawFd> {
            Ok(display.as_ptr() as RawFd)
        }

        fn server_identity(
            &self,
            display: NonNull<c_void>,
            _display_name: &CStr,
        ) -> EngineResult<X11ServerIdentity> {
            self.identities
                .get(&(display.as_ptr() as usize))
                .cloned()
                .ok_or_else(|| {
                    EngineError::new(ErrorCode::InvalidArgument)
                        .with_msg("unknown fake X11 display")
                })
        }

        fn connection_is_alive(&self, _fd: RawFd) -> EngineResult<()> {
            if self.alive.load(Ordering::Relaxed) {
                Ok(())
            } else {
                Err(EngineError::new(ErrorCode::InvalidOperation)
                    .with_msg("owned X11 render connection is closed"))
            }
        }
    }

    #[test]
    fn owner_opens_private_connection_and_closes_it_before_api_drop() {
        let host = pointer(0x1000);
        let owned = pointer(0x2000);
        let api = Arc::new(FakeX11Api::matching(host, owned));
        let events = Arc::clone(&api.events);

        let connection =
            X11RenderConnection::open_with_api(host, Arc::clone(&api) as Arc<dyn X11Api>)
                .expect("matching server must open");

        assert_eq!(connection.display(), owned);
        assert_eq!(api.opens.load(Ordering::Relaxed), 1);
        assert_eq!(&*api.opened_names.lock(), &[b":7.0".to_vec()]);
        assert!(api.closed.lock().is_empty());

        drop(api);
        drop(connection);

        assert_eq!(&*events.lock(), &["close-owned", "api-drop"]);
    }

    #[test]
    fn server_mismatch_closes_candidate_and_returns_error() {
        let host = pointer(0x1000);
        let owned = pointer(0x2000);
        let mut fake = FakeX11Api::matching(host, owned);
        fake.identities.insert(owned.as_ptr() as usize, server(8));
        let api = Arc::new(fake);

        let error = X11RenderConnection::open_with_api(host, Arc::clone(&api) as Arc<dyn X11Api>)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidOperation);
        assert!(error.msg.contains("different X11 server"));
        assert_eq!(&*api.closed.lock(), &[owned.as_ptr() as usize]);
        assert!(
            !api.closed.lock().contains(&(host.as_ptr() as usize)),
            "Migo must never close the host connection"
        );
    }

    #[test]
    fn open_failure_names_the_requested_display_without_closing_host() {
        let host = pointer(0x1000);
        let owned = pointer(0x2000);
        let mut fake = FakeX11Api::matching(host, owned);
        fake.opened = None;
        let api = Arc::new(fake);

        let error = X11RenderConnection::open_with_api(host, Arc::clone(&api) as Arc<dyn X11Api>)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::RenderInitializeError);
        assert_eq!(error.detail.as_deref(), Some(":7.0"));
        assert!(api.closed.lock().is_empty());
    }

    #[test]
    fn reuse_requires_a_live_connection_to_the_same_server() {
        let host = pointer(0x1000);
        let replacement = pointer(0x3000);
        let owned = pointer(0x2000);
        let mut fake = FakeX11Api::matching(host, owned);
        fake.identities
            .insert(replacement.as_ptr() as usize, server(7));
        let api = Arc::new(fake);
        let connection =
            X11RenderConnection::open_with_api(host, Arc::clone(&api) as Arc<dyn X11Api>).unwrap();

        unsafe { connection.supports_host_display(replacement) }
            .expect("same live server must be reusable");

        api.alive.store(false, Ordering::Relaxed);
        let error = unsafe { connection.supports_host_display(replacement) }.unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidOperation);
    }

    #[test]
    #[ignore = "requires an explicit X11 server; run through xvfb-run"]
    fn native_owned_connection_round_trip() {
        let display_name = std::env::var("DISPLAY").expect("DISPLAY must be set by the test gate");
        let display_name = CString::new(display_name).expect("DISPLAY contains no NUL");
        let api = Arc::new(DynamicX11Api::load().expect("load Xlib"));
        let host = api.open_display(&display_name).expect("open host display");

        let context = unsafe { LinuxX11Context::open(host) }.expect("open private render context");
        assert_ne!(
            context.render_display_for_test(),
            host,
            "render and host connections must be distinct"
        );
        unsafe { context.supports_host_display(host) }
            .expect("host display must match private render server");

        let owned_fd = context.connection_fd_for_test();
        let graphics_platform = context.graphics_platform();
        let surface = context.surface(0x2a0_0001, 640, 480);
        drop(context);
        assert_ne!(
            unsafe { libc::fcntl(owned_fd, libc::F_GETFD) },
            -1,
            "the graphics platform and surface must retain the connection"
        );
        drop(graphics_platform);
        assert_ne!(
            unsafe { libc::fcntl(owned_fd, libc::F_GETFD) },
            -1,
            "the surface must retain the connection independently"
        );
        drop(surface);
        assert_eq!(unsafe { libc::fcntl(owned_fd, libc::F_GETFD) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EBADF),
            "dropping the final owner must close the render fd"
        );

        api.close_display(host);
    }
}
