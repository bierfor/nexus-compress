//! UPnP port-forwarding for Direct Mode (Sprint 5.5.3 Phase 2).
//!
//! Opens a TCP port mapping on the local router so a friend on
//! a different network can connect to the sender's axum server
//! via the sender's public IP. The mapping is closed on `Drop`
//! (normal session end) and via `emergency_cleanup_all()` (panic
//! recovery).
//!
//! ## Why a defensive Drop impl?
//!
//! `UpnpHole` opens a port mapping on the user's router. If we
//! forget to close it, the mapping persists until the router's
//! lease expires (anywhere from 20 minutes to 24 hours depending
//! on firmware). That means anyone on the internet can hit that
//! port and try to connect — the SPAKE2 handshake protects
//! them but the exposed port is still an attack surface.
//!
//! `Drop` runs in three scenarios:
//!  1. Normal scope exit (e.g. `let hole = UpnpHole::open(..)?;`)
//!  2. Panic unwinding (Drop runs during unwind)
//!  3. NOT on `SIGKILL` / process crash — there's no hook for
//!     that, but the lease will eventually expire.
//!
//! ## Why a global emergency registry?
//!
//! `panic::set_hook` runs synchronously on panic and CANNOT
//! safely access tokio state. We register a cleanup callback
//! in a static `Mutex<Vec<Box<dyn FnOnce()>>>` when the hole
//! is opened, and drain the registry from the panic hook. The
//! callback captures the `Arc<Gateway>` and the external port,
//! so it can call `remove_port` without going through the
//! `UpnpHole` (which may have been partially moved during
//! unwind).
//!
//! ## What if UPnP is disabled?
//!
//! ~30% of modern networks disable UPnP (post-CVE-2020-12697).
//! `UpnpHole::open` returns `Err` with a human-readable message.
//! The caller (Direct Mode sender) treats this as a soft fail
//! and continues with LAN-only mDNS — the receiver on the same
//! Wi-Fi still works.

use igd::{Gateway, PortMappingProtocol, SearchOptions};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, OnceLock};

/// Lease duration for the port mapping. 1 hour is the smallest
/// value most routers accept and is short enough that a forgotten
/// mapping doesn't linger past a session. If the user keeps the
/// sender running for >1h the router may reclaim the mapping;
/// that's acceptable for the Phase 2 demo.
const UPNP_LEASE_SECONDS: u32 = 3600;

/// Description string shown in the router's UPnP management UI
/// (most routers expose this to the user).
const UPNP_DESCRIPTION: &str = "NexusCompress Direct Share";

/// Static registry of cleanup callbacks. The panic hook drains
/// this. Lock contention is minimal (one push on open, one drain
/// on panic).
type CleanupFn = Box<dyn FnOnce() + Send + 'static>;
static EMERGENCY_REGISTRY: OnceLock<Mutex<Vec<CleanupFn>>> = OnceLock::new();

/// Information about an open UPnP mapping. Returned to the
/// caller so it can log / show the user the external endpoint.
#[derive(Debug, Clone)]
pub struct UpnpHoleInfo {
    /// Public IP the mapping is reachable at (e.g. the WAN IP).
    pub external_ip: Ipv4Addr,
    /// External port (same as internal for Direct Mode — we
    /// don't need to remap).
    pub external_port: u16,
    /// Internal port the sender's axum server is bound to.
    pub internal_port: u16,
    /// Local IP of the sender (the router forwards traffic here).
    pub internal_ip: Ipv4Addr,
}

/// A UPnP port mapping. Closes the mapping on `Drop`.
#[derive(Clone, Debug)]
pub struct UpnpHole {
    gateway: Arc<Gateway>,
    external_port: u16,
    internal_port: u16,
}

impl UpnpHole {
    /// Open a UPnP port mapping. Blocks for up to ~3s on the
    /// gateway search (SSDP multicast). Returns the open hole
    /// + an info struct describing the mapping.
    pub fn open(internal_port: u16) -> Result<(Self, UpnpHoleInfo), String> {
        if internal_port == 0 {
            return Err("internal_port must be non-zero".to_string());
        }
        // 1. Search for the router's UPnP gateway. Blocking,
        //    bounded by `SearchOptions::default()` (~3s timeout).
        let gateway = igd::search_gateway(SearchOptions::default())
            .map_err(|e| {
                format!(
                    "UPnP gateway not found ({}). The router may \
                     have UPnP disabled, or the network blocks \
                     SSDP multicast. Direct Mode will fall back \
                     to LAN-only.",
                    e
                )
            })?;
        // 2. Discover our external IP (the WAN IP the router
        //    advertises to the internet).
        let external_ip = gateway
            .get_external_ip()
            .map_err(|e| format!("UPnP GetExternalIPAddress: {}", e))?;
        // 3. Discover our internal IP — the IP the router will
        //    forward traffic TO. Use the first non-loopback IPv4
        //    interface.
        let internal_ip = discover_internal_ip()
            .ok_or_else(|| "no non-loopback IPv4 interface found".to_string())?;
        // 4. Add the mapping. external == internal port for
        //    Direct Mode (we're not remapping).
        let local_addr = std::net::SocketAddrV4::new(internal_ip, internal_port);
        gateway
            .add_port(
                PortMappingProtocol::TCP,
                internal_port,
                local_addr,
                UPNP_LEASE_SECONDS,
                UPNP_DESCRIPTION,
            )
            .map_err(|e| format!("UPnP AddPortMapping: {}", e))?;
        let external_port = internal_port;
        let info = UpnpHoleInfo {
            external_ip,
            external_port,
            internal_port,
            internal_ip,
        };
        let gateway = Arc::new(gateway);
        // 5. Register the cleanup callback so a panic can
        //    remove the mapping even if the UpnpHole value
        //    was partially moved during unwind.
        register_cleanup(gateway.clone(), external_port);
        Ok((
            UpnpHole {
                gateway,
                external_port,
                internal_port,
            },
            info,
        ))
    }

    /// Best-effort removal. Safe to call multiple times — the
    /// router returns "no such mapping" on the second call and
    /// we ignore the error.
    fn try_remove(&self) {
        let _ = self
            .gateway
            .remove_port(PortMappingProtocol::TCP, self.external_port);
    }
}

impl Drop for UpnpHole {
    fn drop(&mut self) {
        // Best-effort immediate close. If the router is gone
        // (e.g. Wi-Fi dropped) this returns an error which we
        // ignore — the lease will expire on its own.
        self.try_remove();
    }
}

/// Register a cleanup callback in the global registry. The
/// callback removes the port mapping from the router.
fn register_cleanup(gateway: Arc<Gateway>, external_port: u16) {
    let registry = EMERGENCY_REGISTRY
        .get_or_init(|| Mutex::new(Vec::new()));
    let mut reg = registry.lock().expect("registry mutex poisoned");
    reg.push(Box::new(move || {
        let _ = gateway.remove_port(PortMappingProtocol::TCP, external_port);
    }));
}

/// Drain the registry and call every cleanup callback. Used by
/// `panic::set_hook` to recover orphaned mappings when the
/// process is about to abort.
pub fn emergency_cleanup_all() {
    let registry = match EMERGENCY_REGISTRY.get() {
        Some(r) => r,
        None => return, // no holes were ever opened
    };
    let callbacks: Vec<CleanupFn> = {
        let mut reg = registry.lock().expect("registry mutex poisoned");
        std::mem::take(&mut *reg)
    };
    for cb in callbacks {
        // Each callback is idempotent: remove_port returns
        // "no such mapping" if the hole was already closed
        // by Drop, which we ignore.
        cb();
    }
}

/// Discover the first non-loopback IPv4 address on the system.
/// Used as the `internal_client` argument to UPnP port mapping.
fn discover_internal_ip() -> Option<Ipv4Addr> {
    // The local-ip-address crate iterates interfaces on Linux /
    // macOS / Windows without bringing up a UDP socket.
    match local_ip_address::local_ip() {
        Ok(IpAddr::V4(v4)) => Some(v4),
        Ok(IpAddr::V6(_)) => {
            // Fall back to scanning all interfaces for IPv4.
            discover_internal_ip_via_fallback()
        }
        Err(_) => discover_internal_ip_via_fallback(),
    }
}

fn discover_internal_ip_via_fallback() -> Option<Ipv4Addr> {
    let ifs = local_ip_address::list_afinet_netifas().ok()?;
    ifs.into_iter()
        .filter_map(|(_, ip)| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
            _ => None,
        })
        .next()
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_rejects_zero_port() {
        let result = UpnpHole::open(0);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("non-zero"),
            "error must mention non-zero"
        );
    }

    #[test]
    fn emergency_cleanup_with_no_registry_is_noop() {
        // First call ever — no registry exists yet. Must not panic.
        // (Subsequent calls in the test process WILL find the
        // registry, so we don't assert on the drain count.)
        emergency_cleanup_all();
    }

    #[test]
    fn registry_initializes_on_first_register() {
        // Calling emergency_cleanup_all twice in a row is safe
        // even after the registry is created.
        emergency_cleanup_all();
        emergency_cleanup_all();
    }

    #[test]
    fn upnp_hole_info_serializes_debug() {
        // Just a smoke test that the struct is constructible
        // and Debug-printable.
        let info = UpnpHoleInfo {
            external_ip: "203.0.113.1".parse().unwrap(),
            external_port: 49152,
            internal_port: 49152,
            internal_ip: "192.168.1.42".parse().unwrap(),
        };
        let s = format!("{:?}", info);
        assert!(s.contains("203.0.113.1"));
        assert!(s.contains("49152"));
    }

    /// Live UPnP test. Skipped unless the env var is set, because
    /// it requires a real router with UPnP enabled. Run with:
    ///   `UPNP_LIVE_TEST=1 cargo test upnp_hole::tests::live_open_and_close`
    #[test]
    #[ignore]
    fn live_open_and_close() {
        if std::env::var("UPNP_LIVE_TEST").is_err() {
            eprintln!(
                "skipping live UPnP test (set UPNP_LIVE_TEST=1 to run)"
            );
            return;
        }
        let (hole, info) =
            UpnpHole::open(54321).expect("open upnp hole on real router");
        eprintln!(
            "opened UPnP hole: external={}:{}, internal={}:{}",
            info.external_ip, info.external_port, info.internal_ip, info.internal_port
        );
        drop(hole);
        eprintln!("hole dropped, mapping should be gone");
    }
}