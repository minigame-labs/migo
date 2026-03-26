use std::net::SocketAddr;

/// Delay inserted before each poll iteration when the app is backgrounded.
pub(super) const BACKGROUND_THROTTLE: std::time::Duration =
    std::time::Duration::from_millis(500);

/// Resolve a host:port string to the first SocketAddr.
///
/// Rejects addresses in private/loopback/link-local ranges to prevent SSRF.
/// Checks ALL resolved addresses — if any points to a blocked range the
/// entire resolution is rejected (prevents mixed public/private DNS responses).
pub(super) async fn resolve_first(addr: &str) -> Result<SocketAddr, std::io::Error> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(addr).await?.collect();
    let first = *addrs.first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            format!("could not resolve '{}'", addr),
        )
    })?;
    for resolved in &addrs {
        if super::address_filter::is_blocked_address(resolved) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "connection to {} is not allowed (private/loopback address)",
                    resolved.ip()
                ),
            ));
        }
    }
    Ok(first)
}

/// Return "IPv4" or "IPv6" string for a SocketAddr.
pub(super) fn addr_family(addr: &SocketAddr) -> String {
    match addr {
        SocketAddr::V4(_) => "IPv4".to_string(),
        SocketAddr::V6(_) => "IPv6".to_string(),
    }
}
