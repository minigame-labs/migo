use std::net::SocketAddr;

use deno_error::JsErrorBox;

/// Delay inserted before each poll iteration when the app is backgrounded.
pub(super) const BACKGROUND_THROTTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// Narrow a JS-supplied port (arrives as `u32`) to `u16`, rejecting
/// out-of-range values instead of silently wrapping. `70000 as u16`
/// is `4464` — a *different* port — so raw sockets must reject rather
/// than connect somewhere the caller never asked for.
pub(super) fn checked_port(port: u32) -> Result<u16, JsErrorBox> {
    u16::try_from(port)
        .map_err(|_| JsErrorBox::type_error(format!("port {} out of range (0-65535)", port)))
}

/// Build a `host:port` string for `tokio::net::lookup_host`, bracketing
/// bare IPv6 literals (`::1` -> `[::1]:port`). Without the brackets the
/// literal's own colons make the string ambiguous and resolution fails,
/// so raw IPv6-literal connect/send targets would never work.
pub(super) fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_port_accepts_full_u16_range() {
        assert_eq!(checked_port(0).unwrap(), 0);
        assert_eq!(checked_port(80).unwrap(), 80);
        assert_eq!(checked_port(65535).unwrap(), 65535);
    }

    #[test]
    fn checked_port_rejects_out_of_range_instead_of_wrapping() {
        // Regression: `70000 as u16` silently becomes 4464. Reject it.
        assert!(checked_port(65536).is_err());
        assert!(checked_port(70000).is_err());
        assert!(checked_port(u32::MAX).is_err());
    }

    #[test]
    fn join_host_port_brackets_bare_ipv6_literal() {
        assert_eq!(join_host_port("::1", 80), "[::1]:80");
        assert_eq!(
            join_host_port("2001:db8::1", 443),
            "[2001:db8::1]:443"
        );
    }

    #[test]
    fn join_host_port_leaves_already_bracketed_ipv6() {
        assert_eq!(join_host_port("[::1]", 80), "[::1]:80");
    }

    #[test]
    fn join_host_port_passes_through_ipv4_and_hostname() {
        assert_eq!(join_host_port("1.2.3.4", 80), "1.2.3.4:80");
        assert_eq!(join_host_port("example.com", 443), "example.com:443");
    }
}
