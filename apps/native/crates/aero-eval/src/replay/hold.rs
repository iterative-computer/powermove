//! The replay's network boundary on this Mac (no VM): the replay server's
//! port is held on every interface for as long as the run goes, so the one
//! TCP port a confined run may reach (Seatbelt's `localhost` matches loopback
//! and this Mac's own addresses alike) leads only to the replay server.
//!
//! On BSD sockets a port held on the wildcard address alone can still be
//! taken on a specific address by a socket with `SO_REUSEADDR` or
//! `SO_REUSEPORT` (measured on this Mac), so the hold binds the loopback
//! address, both wildcards and every address of every interface, each
//! without `SO_REUSEPORT`: an exact duplicate then needs `SO_REUSEPORT` on
//! both sockets and is refused. [`PortHold::prove`] tries every way to take
//! the port before a replay starts, and a watchdog holds addresses that
//! appear while the run goes (a VPN coming up) or marks the hold broken when
//! one cannot be held. Connections that reach a holder are dropped at once;
//! only loopback peers reach the replay.
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the watchdog looks for new addresses.
const RESCAN: Duration = Duration::from_millis(200);
/// How often holders drop connections that reached them.
const DROP_EVERY: Duration = Duration::from_millis(20);

/// Every address of every interface of this machine, IPv6 link-local ones
/// with their scope.
pub fn local_addresses() -> Vec<SocketAddr> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list we free with freeifaddrs below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut at = head;
    while !at.is_null() {
        // SAFETY: a node of the list getifaddrs returned.
        let ifa = unsafe { &*at };
        at = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: ifa_addr points at a sockaddr of the family it names.
        match unsafe { (*ifa.ifa_addr).sa_family } as libc::c_int {
            libc::AF_INET => {
                // SAFETY: AF_INET means sockaddr_in.
                let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
                out.push(SocketAddr::new(IpAddr::V4(ip), 0));
            }
            libc::AF_INET6 => {
                // SAFETY: AF_INET6 means sockaddr_in6.
                let sa = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                let mut b = sa.sin6_addr.s6_addr;
                let mut scope = sa.sin6_scope_id;
                // KAME: link-local addresses may carry their scope in bytes 2–3.
                if b[0] == 0xfe && (b[1] & 0xc0) == 0x80 && (b[2] != 0 || b[3] != 0) {
                    if scope == 0 {
                        scope = u32::from(u16::from_be_bytes([b[2], b[3]]));
                    }
                    b[2] = 0;
                    b[3] = 0;
                }
                out.push(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(b),
                    0,
                    0,
                    scope,
                )));
            }
            _ => {}
        }
    }
    // SAFETY: the list from getifaddrs above.
    unsafe { libc::freeifaddrs(head) };
    out.sort();
    out.dedup();
    out
}

fn with_port(a: SocketAddr, port: u16) -> SocketAddr {
    let mut a = a;
    a.set_port(port);
    a
}

fn wildcards(port: u16) -> [SocketAddr; 2] {
    [
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
    ]
}

/// A listening holder: `SO_REUSEADDR` (so specific addresses bind beside the
/// wildcard), never `SO_REUSEPORT`, IPv6 sockets IPv6-only.
fn holder(addr: SocketAddr) -> std::io::Result<Socket> {
    let s = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        s.set_only_v6(true)?;
    }
    s.set_reuse_address(true)?;
    s.bind(&addr.into())?;
    s.listen(16)?;
    s.set_nonblocking(true)?;
    Ok(s)
}

fn unusable(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(libc::EADDRNOTAVAIL)
}

struct Shared {
    stop: AtomicBool,
    broken: Mutex<Option<String>>,
    holders: Mutex<Vec<(SocketAddr, Socket)>>,
}

/// One TCP port held on every interface; the replay serves on its loopback
/// listener.
pub struct PortHold {
    pub port: u16,
    listener: Option<TcpListener>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PortHold {
    /// Pick a free port and hold it everywhere (a few ports are tried when
    /// another listener already has the picked one somewhere).
    pub fn take() -> Result<Self, String> {
        let mut last = String::new();
        for _ in 0..16 {
            match Self::try_take() {
                Ok(h) => return Ok(h),
                Err(e) => last = e,
            }
        }
        Err(format!("could not hold a port on every interface: {last}"))
    }

    fn try_take() -> Result<Self, String> {
        let lo = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
            .and_then(|s| {
                s.bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())?;
                s.listen(64)?;
                Ok(s)
            })
            .map_err(|e| e.to_string())?;
        let port = lo
            .local_addr()
            .ok()
            .and_then(|a| a.as_socket())
            .map(|a| a.port())
            .ok_or("no port")?;
        let mut holders = Vec::new();
        let mut want: Vec<SocketAddr> = wildcards(port).to_vec();
        want.extend(
            local_addresses()
                .into_iter()
                .map(|a| with_port(a, port))
                .filter(|a| a.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)),
        );
        for a in want {
            match holder(a) {
                Ok(s) => holders.push((a, s)),
                Err(e) if unusable(&e) => {}
                Err(e) => return Err(format!("{a}: {e}")),
            }
        }
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            broken: Mutex::new(None),
            holders: Mutex::new(holders),
        });
        let s = shared.clone();
        let thread = std::thread::spawn(move || watchdog(&s, port));
        Ok(Self {
            port,
            listener: Some(lo.into()),
            shared,
            thread: Some(thread),
        })
    }

    /// The loopback listener the replay serves on (taken once).
    pub fn listener(&mut self) -> Option<TcpListener> {
        self.listener.take()
    }

    /// Try every way another socket could take the port: on loopback, both
    /// wildcards (a dual-stack socket included) and every interface address,
    /// with no option, `SO_REUSEADDR`, `SO_REUSEPORT` and both. Each must be
    /// refused.
    pub fn prove(&self) -> Result<(), String> {
        if let Some(b) = self.broken() {
            return Err(b);
        }
        let port = self.port;
        let mut tries: Vec<(SocketAddr, bool)> =
            wildcards(port).iter().map(|a| (*a, true)).collect();
        tries.push((SocketAddr::from((Ipv4Addr::LOCALHOST, port)), true));
        tries.push((SocketAddr::from((Ipv6Addr::LOCALHOST, port)), true));
        tries.extend(
            local_addresses()
                .into_iter()
                .map(|a| (with_port(a, port), true)),
        );
        // A dual-stack IPv6 socket on the wildcard and on mapped loopback.
        tries.push((SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)), false));
        tries.push((
            SocketAddr::from((Ipv4Addr::LOCALHOST.to_ipv6_mapped(), port)),
            false,
        ));
        for (a, v6only) in tries {
            for (reuse_addr, reuse_port) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                match try_bind(a, v6only, reuse_addr, reuse_port) {
                    Ok(()) => {
                        return Err(format!(
                            "another socket could take port {port} on {a} (SO_REUSEADDR {reuse_addr}, SO_REUSEPORT {reuse_port})"
                        ));
                    }
                    Err(e) if e.raw_os_error() == Some(libc::EADDRINUSE) || unusable(&e) => {}
                    Err(e) => return Err(format!("{a}: {e}")),
                }
            }
        }
        Ok(())
    }

    /// Why the hold stopped covering every interface, if it did.
    pub fn broken(&self) -> Option<String> {
        self.shared.broken.lock().unwrap().clone()
    }

    /// The addresses held besides loopback (for reports and tests).
    pub fn held(&self) -> Vec<SocketAddr> {
        self.shared
            .holders
            .lock()
            .unwrap()
            .iter()
            .map(|(a, _)| *a)
            .collect()
    }
}

fn try_bind(
    a: SocketAddr,
    v6only: bool,
    reuse_addr: bool,
    reuse_port: bool,
) -> std::io::Result<()> {
    let s = Socket::new(Domain::for_address(a), Type::STREAM, Some(Protocol::TCP))?;
    if a.is_ipv6() {
        s.set_only_v6(v6only)?;
    }
    s.set_reuse_address(reuse_addr)?;
    s.set_reuse_port(reuse_port)?;
    s.bind(&a.into())?;
    s.listen(1)
}

fn watchdog(s: &Shared, port: u16) {
    let mut scanned = Instant::now();
    while !s.stop.load(Ordering::SeqCst) {
        {
            let holders = s.holders.lock().unwrap();
            for (_, h) in holders.iter() {
                // Whatever reached a holder is not a loopback replay client.
                while let Ok((c, _)) = h.accept() {
                    drop(c);
                }
            }
        }
        if scanned.elapsed() >= RESCAN {
            scanned = Instant::now();
            let mut holders = s.holders.lock().unwrap();
            for a in local_addresses() {
                let a = with_port(a, port);
                if a.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST) || holders.iter().any(|(h, _)| *h == a)
                {
                    continue;
                }
                match holder(a) {
                    Ok(h) => holders.push((a, h)),
                    Err(e) if unusable(&e) => {}
                    Err(e) => {
                        s.broken.lock().unwrap().get_or_insert(format!(
                            "{a} appeared while the run went and port {port} could not be held there: {e}"
                        ));
                    }
                }
            }
        }
        std::thread::sleep(DROP_EVERY);
    }
}

impl Drop for PortHold {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
