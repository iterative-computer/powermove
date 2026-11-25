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
}

#[derive(Debug, Default)]
pub struct MemorySecrets(pub Mutex<BTreeMap<(String, String), Vec<u8>>>);
impl Secrets for MemorySecrets {
    fn read(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().unwrap().get(&(service.into(), account.into())).cloned())
    }
    fn write(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap().insert((service.into(), account.into()), bytes.to_vec());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(&(service.into(), account.into()));
        Ok(())
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
            return Err(format!("The loopback client reaches 127.0.0.1 only, not {}.", url.origin().ascii_serialization()));
        }
        let mut stream = TcpStream::connect(("127.0.0.1", url.port().unwrap_or(80))).map_err(|e| e.to_string())?;
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
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("No HTTP status line.")?;
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
                    .map(|(from, to)| (from.trim_end_matches('/').to_owned(), to.trim_end_matches('/').to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        if map.is_empty() { inner } else { Arc::new(Self { map, inner }) }
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
            Some(url) => Loopback.stream(&NetRequest { url: &url, ..*request }, stop, on),
            None => self.inner.stream(request, stop, on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reroute_maps_only_listed_loopback_targets() {
        let vars = BTreeMap::from([(
            "PM_AGENT_FAKES".to_owned(),
            "https://chatgpt.com=http://127.0.0.1:9 https://api.anthropic.com=https://elsewhere".to_owned(),
        )]);
        let net = Reroute::from_env(&vars, Arc::new(Loopback));
        let text = format!("{net:?}");
        assert!(text.contains("https://chatgpt.com") && !text.contains("elsewhere"), "{text}");
        let probe = Reroute { map: vec![("https://chatgpt.com".into(), "http://127.0.0.1:9".into())], inner: Arc::new(Loopback) };
        assert_eq!(
            probe.target("https://chatgpt.com/backend-api/codex/responses").as_deref(),
            Some("http://127.0.0.1:9/backend-api/codex/responses")
        );
        assert!(probe.target("https://chatgpt.com.evil/x").is_none());
    }
}
