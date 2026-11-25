//! What Settings › Providers needs from the app: the Keychain (`Secrets`) and an
//! HTTP client that streams (`Net`, the system HTTP stack in the app). Plus a
//! plain loopback client for the local fakes, and, in development builds
//! only, a reroute of provider origins to those fakes. Nothing here logs a
//! value it carries.
use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Generic-password items by service and account (the macOS Keychain; a
/// memory map in tests).
pub trait Secrets: Send + Sync + std::fmt::Debug {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
    /// Codex's item, read the way Codex 0.169 reads it (its macOS keyring
    /// backend opens the default User-domain keychain, not the search list).
    fn read_codex(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        self.read(service, account)
    }
}

#[derive(Debug, Default)]
pub struct MemorySecrets(pub Mutex<BTreeMap<(String, String), Vec<u8>>>);
impl Secrets for MemorySecrets {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned())
    }
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), bytes.to_vec());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .remove(&(service.into(), account.into()));
        Ok(())
    }
}

/// A stand-in for the Keychain in one JSON file, for development builds and
/// tests against the local fakes (`PM_AGENT_FAKE_KEYCHAIN`): Codex's fake and
/// Powermove share it across processes, as they share one Keychain item for
/// real. It never touches a real keychain and never shows a dialog. Writers
/// hold an exclusive lock on `<file>.lock`, taken within two seconds or the
/// call fails; the file is replaced whole, so a reader never sees half a write.
#[derive(Debug)]
pub struct FakeKeychain(pub std::path::PathBuf);
type Items = serde_json::Map<String, serde_json::Value>;
/// Which Keychain a process may use. A development build launched against the
/// local fakes (any of `PM_AGENT_FAKE_KEYCHAIN`, `PM_AGENT_FAKES`,
/// `PM_CODEX_KEYRING_SERVICE`) uses only its fake store, and a missing or
/// invalid store setting refuses every Keychain call: it never falls through
/// to the real Keychain.
#[derive(Debug)]
pub enum KeychainMode {
    Real,
    Fake(FakeKeychain),
    Refused(String),
}
pub fn keychain_mode(vars: &BTreeMap<String, String>) -> KeychainMode {
    if !cfg!(debug_assertions) {
        return KeychainMode::Real;
    }
    let lane = [
        "PM_AGENT_FAKE_KEYCHAIN",
        "PM_AGENT_FAKES",
        "PM_CODEX_KEYRING_SERVICE",
    ]
    .iter()
    .any(|name| vars.contains_key(*name));
    if !lane {
        return KeychainMode::Real;
    }
    let path = vars
        .get("PM_AGENT_FAKE_KEYCHAIN")
        .map(std::path::PathBuf::from);
    match path {
        Some(path)
            if path.is_absolute()
                && path.extension().is_some_and(|e| e == "json")
                && path.parent().is_some_and(std::path::Path::is_dir) =>
        {
            KeychainMode::Fake(FakeKeychain(path))
        }
        _ => KeychainMode::Refused(
            "This development build runs against local fakes without a valid fake Keychain \
             (PM_AGENT_FAKE_KEYCHAIN, an absolute .json path in an existing folder); \
             it will not touch the real Keychain."
                .into(),
        ),
    }
}
/// A store that answers every call with why the Keychain is closed.
#[derive(Debug)]
pub struct RefusedKeychain(pub String);
impl Secrets for RefusedKeychain {
    fn read(&self, _: &str, _: &str) -> Result<Option<Vec<u8>>, String> {
        Err(self.0.clone())
    }
    fn write(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err(self.0.clone())
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
}
impl FakeKeychain {
    /// The development switch's fake store, when the environment names one.
    pub fn from_env(vars: &BTreeMap<String, String>) -> Option<Self> {
        vars.get("PM_AGENT_FAKE_KEYCHAIN")
            .filter(|p| cfg!(debug_assertions) && p.ends_with(".json"))
            .map(|p| Self(p.into()))
    }
    fn key(service: &str, account: &str) -> String {
        format!("{service}\t{account}")
    }
    fn with<T>(&self, change: impl FnOnce(&mut Items) -> (T, bool)) -> Result<T, String> {
        use std::os::fd::AsRawFd as _;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.0.with_extension("json.lock"))
            .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        // SAFETY: flock on a descriptor this function owns.
        while unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            if std::time::Instant::now() >= deadline {
                return Err("The fake keychain is busy.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut items: Items = match std::fs::read(&self.0) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Items::new(),
            Err(e) => return Err(e.to_string()),
        };
        let (out, changed) = change(&mut items);
        if changed {
            let tmp = self.0.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&items).expect("items"))
                .and_then(|()| std::fs::rename(&tmp, &self.0))
                .map_err(|e| e.to_string())?;
        }
        Ok(out)
    }
}
impl Secrets for FakeKeychain {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        self.with(|items| {
            let found = items
                .get(&Self::key(service, account))
                .and_then(|v| v.as_str());
            (found.map(|v| v.as_bytes().to_vec()), false)
        })
    }
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())?
            .to_owned();
        self.with(|items| {
            items.insert(Self::key(service, account), text.into());
            ((), true)
        })
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.with(|items| ((), items.remove(&Self::key(service, account)).is_some()))
    }
}

/// What a streamed response delivers: its status once, then its bytes.
pub enum Chunk<'a> {
    Status(u16),
    Data(&'a [u8]),
}
pub struct NetRequest<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
}
/// The app's HTTP client. It never follows a redirect with the headers it
/// was given.
pub trait Net: Send + Sync + std::fmt::Debug {
    fn stream(
        &self,
        request: &NetRequest<'_>,
        stop: &AtomicBool,
        on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
    ) -> Result<(), String>;
}

/// The whole body of a request, with its status.
pub fn fetch(net: &dyn Net, request: &NetRequest<'_>) -> Result<(u16, Vec<u8>), String> {
    let (mut status, mut body) = (0, Vec::new());
    net.stream(request, &AtomicBool::new(false), &mut |chunk| {
        match chunk {
            Chunk::Status(code) => status = code,
            Chunk::Data(bytes) if body.len() < 8 << 20 => body.extend_from_slice(bytes),
            Chunk::Data(_) => return Err("The response is too large.".into()),
        }
        Ok(())
    })?;
    Ok((status, body))
}

/// How long a model's stream may go without a byte before Powermove gives up
/// on it: Codex's stream idle timeout. A provider that hangs mid-reply ends
/// the turn with a clear error instead of leaving it working for an hour.
pub const STREAM_IDLE: std::time::Duration = std::time::Duration::from_secs(300);

/// `inner`, stopped when no byte has arrived for `after` (headers count).
#[derive(Debug)]
pub struct Idle {
    pub inner: Arc<dyn Net>,
    pub after: std::time::Duration,
}
impl Net for Idle {
    fn stream(
        &self,
        request: &NetRequest<'_>,
        stop: &AtomicBool,
        on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        use std::time::Instant;
        let last = Mutex::new(Instant::now());
        let (ended, quiet, halt) = (
            AtomicBool::new(false),
            AtomicBool::new(false),
            AtomicBool::new(false),
        );
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !ended.load(Ordering::Acquire) {
                    if stop.load(Ordering::Acquire) {
                        halt.store(true, Ordering::Release);
                    } else if last.lock().unwrap().elapsed() >= self.after {
                        quiet.store(true, Ordering::Release);
                        halt.store(true, Ordering::Release);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            });
            let result = self.inner.stream(request, &halt, &mut |chunk| {
                *last.lock().unwrap() = Instant::now();
                on(chunk)
            });
            ended.store(true, Ordering::Release);
            result
        });
        if quiet.load(Ordering::Acquire) {
            let seconds = self.after.as_secs();
            let span = if seconds >= 60 && seconds % 60 == 0 {
                format!("{} minutes", seconds / 60)
            } else {
                format!("{:.1} seconds", self.after.as_secs_f64())
            };
            return Err(format!(
                "The model stopped responding: nothing arrived for {span}. Try again."
            ));
        }
        result
    }
}

/// Plain HTTP/1.1 to 127.0.0.1 only: the local fakes.
#[derive(Debug)]
pub struct Loopback;
impl Net for Loopback {
    fn stream(
        &self,
        request: &NetRequest<'_>,
        stop: &AtomicBool,
        on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let url = url::Url::parse(request.url).map_err(|e| e.to_string())?;
        if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
            return Err(format!(
                "The loopback client reaches 127.0.0.1 only, not {}.",
                url.origin().ascii_serialization()
            ));
        }
        let mut stream = TcpStream::connect(("127.0.0.1", url.port().unwrap_or(80)))
            .map_err(|e| e.to_string())?;
        let target = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        let mut head = format!(
            "{} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n",
            request.method,
            request.body.len()
        );
        for (k, v) in request.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(request.body))
            .map_err(|e| e.to_string())?;
        let fd = std::os::fd::AsRawFd::as_raw_fd(&stream);
        let mut reader = BufReader::new(stream);
        // A held answer must not outlast a stop (a sign-out, a cancelled turn).
        let ready = |reader: &BufReader<TcpStream>| -> Result<(), String> {
            while reader.buffer().is_empty() {
                if stop.load(Ordering::Acquire) {
                    return Err("Stopped.".into());
                }
                let mut poll = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: one pollfd for a descriptor this function owns.
                if unsafe { libc::poll(&mut poll, 1, 100) } != 0 {
                    break;
                }
            }
            Ok(())
        };
        ready(&reader)?;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let status = line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or("No HTTP status line.")?;
        let mut chunked = false;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).map_err(|e| e.to_string())?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':')
                && k.eq_ignore_ascii_case("transfer-encoding")
                && v.trim().eq_ignore_ascii_case("chunked")
            {
                chunked = true;
            }
        }
        on(Chunk::Status(status))?;
        let mut buf = [0u8; 8192];
        if chunked {
            loop {
                let mut size = String::new();
                reader.read_line(&mut size).map_err(|e| e.to_string())?;
                let n = usize::from_str_radix(size.trim(), 16).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                let mut data = vec![0; n];
                reader.read_exact(&mut data).map_err(|e| e.to_string())?;
                on(Chunk::Data(&data))?;
                reader.read_line(&mut size).map_err(|e| e.to_string())?;
                if stop.load(Ordering::Acquire) {
                    break;
                }
            }
            return Ok(());
        }
        while !stop.load(Ordering::Acquire) {
            ready(&reader)?;
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            on(Chunk::Data(&buf[..n]))?;
        }
        Ok(())
    }
}

/// Development builds only: provider origins answered by local fakes
/// (`PM_AGENT_FAKES="https://chatgpt.com=http://127.0.0.1:9101 …"`). The
/// request keeps the headers its canonical origin was given, so a fake can
/// check them; every other origin goes to `inner`.
#[derive(Debug)]
pub struct Reroute {
    pub map: Vec<(String, String)>,
    pub inner: Arc<dyn Net>,
}
impl Reroute {
    pub fn from_env(vars: &BTreeMap<String, String>, inner: Arc<dyn Net>) -> Arc<dyn Net> {
        let map: Vec<(String, String)> = vars
            .get("PM_AGENT_FAKES")
            .filter(|_| cfg!(debug_assertions))
            .map(|spec| {
                spec.split_whitespace()
                    .filter_map(|pair| pair.split_once('='))
                    .filter(|(_, to)| to.starts_with("http://127.0.0.1:"))
                    .map(|(from, to)| {
                        (
                            from.trim_end_matches('/').to_owned(),
                            to.trim_end_matches('/').to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if map.is_empty() {
            inner
        } else {
            Arc::new(Self { map, inner })
        }
    }
    /// The URL a fake answers, if one stands in for `url`'s origin.
    pub fn target(&self, url: &str) -> Option<String> {
        self.map.iter().find_map(|(from, to)| {
            let rest = url.strip_prefix(from.as_str())?;
            (rest.is_empty() || rest.starts_with(['/', '?'])).then(|| format!("{to}{rest}"))
        })
    }
}
impl Net for Reroute {
    fn stream(
        &self,
        request: &NetRequest<'_>,
        stop: &AtomicBool,
        on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        match self.target(request.url) {
            Some(url) => Loopback.stream(
                &NetRequest {
                    url: &url,
                    ..*request
                },
                stop,
                on,
            ),
            None => self.inner.stream(request, stop, on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The idle stop counts silence, not the whole request: a reply that keeps
    /// trickling in for longer than the limit is never cut.
    #[test]
    fn a_slow_but_steady_stream_outlives_the_idle_limit() {
        #[derive(Debug)]
        struct Trickle;
        impl Net for Trickle {
            fn stream(
                &self,
                _: &NetRequest<'_>,
                stop: &AtomicBool,
                on: &mut dyn FnMut(Chunk<'_>) -> Result<(), String>,
            ) -> Result<(), String> {
                on(Chunk::Status(200))?;
                for _ in 0..8 {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    if stop.load(Ordering::Acquire) {
                        return Err("Stopped.".into());
                    }
                    on(Chunk::Data(b"x"))?;
                }
                Ok(())
            }
        }
        let idle = Idle {
            inner: Arc::new(Trickle),
            after: std::time::Duration::from_millis(200),
        };
        let request = NetRequest {
            method: "POST",
            url: "http://127.0.0.1:9/",
            headers: &[],
            body: b"",
        };
        let mut bytes = 0;
        idle.stream(&request, &AtomicBool::new(false), &mut |chunk| {
            if let Chunk::Data(data) = chunk {
                bytes += data.len();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(bytes, 8);
    }

    /// P1-3: a lane's fake-store setting that is missing or invalid closes
    /// the Keychain instead of reaching the real one.
    #[test]
    fn an_invalid_lane_fake_store_setting_never_reaches_the_real_keychain() {
        let dir = std::env::temp_dir();
        let vars = |pairs: &[(&str, String)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect()
        };
        let refused = |vars: BTreeMap<String, String>| {
            matches!(keychain_mode(&vars), KeychainMode::Refused(_))
        };
        // A typo, a missing suffix, a relative path, a missing folder.
        for bad in [
            dir.join("store").display().to_string(),
            dir.join("store.jsn").display().to_string(),
            "store.json".to_owned(),
            dir.join("no-such-folder-pm/store.json")
                .display()
                .to_string(),
            String::new(),
        ] {
            assert!(
                refused(vars(&[("PM_AGENT_FAKE_KEYCHAIN", bad.clone())])),
                "{bad}"
            );
        }
        // The fakes named without any store at all.
        assert!(refused(vars(&[(
            "PM_AGENT_FAKES",
            "https://chatgpt.com=http://127.0.0.1:9".into()
        )])));
        assert!(refused(vars(&[(
            "PM_CODEX_KEYRING_SERVICE",
            "lane Codex Auth".into()
        )])));
        let ok = dir.join("store.json").display().to_string();
        assert!(matches!(
            keychain_mode(&vars(&[("PM_AGENT_FAKE_KEYCHAIN", ok)])),
            KeychainMode::Fake(_)
        ));
        assert!(matches!(
            keychain_mode(&BTreeMap::new()),
            KeychainMode::Real
        ));
        assert!(RefusedKeychain("closed".into()).read("s", "a").is_err());
    }

    #[test]
    fn the_fake_keychain_keeps_items_across_handles_and_writes_whole() {
        let dir = std::env::temp_dir().join(format!("pm-fake-keychain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("items.json");
        let (a, b) = (FakeKeychain(path.clone()), FakeKeychain(path.clone()));
        assert_eq!(a.read("S", "a").unwrap(), None);
        a.write("S", "a", b"one").unwrap();
        assert_eq!(b.read("S", "a").unwrap().as_deref(), Some(&b"one"[..]));
        let writers: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    FakeKeychain(path)
                        .write("S", &format!("k{i}"), b"v")
                        .unwrap()
                })
            })
            .collect();
        writers.into_iter().for_each(|w| w.join().unwrap());
        assert!((0..8).all(|i| a.read("S", &format!("k{i}")).unwrap().is_some()));
        b.delete("S", "a").unwrap();
        assert_eq!(a.read("S", "a").unwrap(), None);
        let vars = BTreeMap::from([(
            "PM_AGENT_FAKE_KEYCHAIN".to_owned(),
            path.display().to_string(),
        )]);
        assert!(FakeKeychain::from_env(&vars).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reroute_maps_only_listed_loopback_targets() {
        let vars = BTreeMap::from([(
            "PM_AGENT_FAKES".to_owned(),
            "https://chatgpt.com=http://127.0.0.1:9 https://api.anthropic.com=https://elsewhere"
                .to_owned(),
        )]);
        let net = Reroute::from_env(&vars, Arc::new(Loopback));
        let text = format!("{net:?}");
        assert!(
            text.contains("https://chatgpt.com") && !text.contains("elsewhere"),
            "{text}"
        );
        let probe = Reroute {
            map: vec![("https://chatgpt.com".into(), "http://127.0.0.1:9".into())],
            inner: Arc::new(Loopback),
        };
        assert_eq!(
            probe
                .target("https://chatgpt.com/backend-api/codex/responses")
                .as_deref(),
            Some("http://127.0.0.1:9/backend-api/codex/responses")
        );
        assert!(probe.target("https://chatgpt.com.evil/x").is_none());
    }
}
