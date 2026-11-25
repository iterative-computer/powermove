//! One owner for the shared ChatGPT login's refresh exchange. Refresh tokens
//! rotate, and Codex 0.169 makes the OAuth exchange before it takes its own
//! cross-process lock, so two Codex processes (a Codex run's App Server and
//! the one Powermove asks to refresh) can spend the same refresh token. With
//! Settings › Providers on, every Codex process Powermove starts sends its
//! refresh to this broker instead (`CODEX_REFRESH_TOKEN_URL_OVERRIDE`, which
//! Codex 0.169 honours over its own endpoint):
//!
//! - one exchange at a time, process-wide;
//! - an exchange runs to its end even when the process that asked has gone,
//!   and its answer is kept for a while by the refresh token it spent, so a
//!   later ask with the same token (an uncertain outcome, a second process
//!   that read the same item) gets that answer instead of spending the token
//!   again;
//! - a sign-out forgets every kept answer.
//!
//! The broker listens on 127.0.0.1 under a random path, holds answers only in
//! memory, and logs nothing it carries.
use crate::edge::{Net, NetRequest, fetch};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Where Codex 0.169 refreshes a ChatGPT login.
pub const UPSTREAM: &str = "https://auth.openai.com/oauth/token";
/// How long an exchange's answer is kept for a repeat of its refresh token.
const KEEP: Duration = Duration::from_secs(15 * 60);

struct Broker {
    port: u16,
    secret: String,
    /// The network each profile's route forwards through.
    routes: Mutex<HashMap<String, Arc<dyn Net>>>,
    /// Held across an exchange and its bookkeeping: one at a time.
    exchange: Mutex<Kept>,
}
#[derive(Default)]
struct Kept {
    generation: u64,
    answers: HashMap<[u8; 32], (Instant, u16, Vec<u8>)>,
}

static BROKER: OnceLock<Result<Arc<Broker>, String>> = OnceLock::new();

fn broker() -> Result<Arc<Broker>, String> {
    BROKER
        .get_or_init(|| {
            let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let mut random = [0u8; 16];
            getrandom(&mut random)?;
            let broker = Arc::new(Broker {
                port,
                secret: random.iter().map(|b| format!("{b:02x}")).collect(),
                routes: Mutex::default(),
                exchange: Mutex::default(),
            });
            let serving = broker.clone();
            std::thread::Builder::new()
                .name("pm-refresh-broker".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let broker = serving.clone();
                        std::thread::spawn(move || broker.serve(stream));
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(broker)
        })
        .clone()
}
fn getrandom(out: &mut [u8]) -> Result<(), String> {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(out))
        .map_err(|e| format!("No random source: {e}"))
}

/// The refresh URL for Codex processes of this profile, forwarding through
/// `net`.
pub fn url(route: &str, net: Arc<dyn Net>) -> Result<String, String> {
    let broker = broker()?;
    let id = format!("{:x}", Sha256::digest(route.as_bytes()));
    let id = &id[..16];
    broker.routes.lock().unwrap().insert(id.to_owned(), net);
    Ok(format!(
        "http://127.0.0.1:{}/{}/{id}/oauth/token",
        broker.port, broker.secret
    ))
}

/// A sign-out or a new sign-in: no kept answer may serve the old login.
pub fn forget(generation: u64) {
    if let Some(Ok(broker)) = BROKER.get() {
        let mut kept = broker.exchange.lock().unwrap_or_else(|e| e.into_inner());
        kept.generation = generation;
        kept.answers.clear();
    }
}

impl Broker {
    fn serve(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let Some((path, content_type, body)) = read_request(&stream) else {
            return reply(&mut stream, 400, b"{}");
        };
        let route = path
            .strip_prefix(&format!("/{}/", self.secret))
            .and_then(|rest| rest.strip_suffix("/oauth/token"));
        let net = route.and_then(|id| self.routes.lock().unwrap().get(id).cloned());
        let Some(net) = net else {
            return reply(&mut stream, 404, b"{}");
        };
        let route = route.unwrap_or_default();
        let (status, answer) = self.exchange(route, net.as_ref(), &content_type, &body);
        reply(&mut stream, status, &answer);
    }

    /// The upstream answer for this refresh, made at most once per refresh
    /// token while it is kept.
    fn exchange(
        &self,
        route: &str,
        net: &dyn Net,
        content_type: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        let Some(token) = refresh_token(content_type, body) else {
            return (400, br#"{"error":"invalid_request"}"#.to_vec());
        };
        // By the profile's route and the token: two issuers never share answers.
        let key: [u8; 32] = Sha256::digest(format!("{route}\n{token}").as_bytes()).into();
        let mut kept = self.exchange.lock().unwrap_or_else(|e| e.into_inner());
        kept.answers.retain(|_, (at, _, _)| at.elapsed() < KEEP);
        if let Some((_, status, answer)) = kept.answers.get(&key) {
            return (*status, answer.clone());
        }
        let headers = [
            ("Content-Type".to_owned(), content_type.to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ];
        let request = NetRequest {
            method: "POST",
            url: UPSTREAM,
            headers: &headers,
            body,
        };
        match fetch(net, &request) {
            Ok((status, answer)) => {
                // Kept whatever it says: the token is spent either way, and
                // a repeat must hear the same answer, never ask again.
                kept.answers
                    .insert(key, (Instant::now(), status, answer.clone()));
                (status, answer)
            }
            // Nothing reached the issuer, or its answer was lost: say so,
            // and let Codex keep its credentials.
            Err(_) => (503, br#"{"error":"temporarily_unavailable"}"#.to_vec()),
        }
    }
}

fn refresh_token(content_type: &str, body: &[u8]) -> Option<String> {
    let token = if content_type.contains("json") {
        serde_json::from_slice::<serde_json::Value>(body).ok()?["refresh_token"]
            .as_str()?
            .to_owned()
    } else {
        url::form_urlencoded::parse(body)
            .find(|(k, _)| k == "refresh_token")?
            .1
            .into_owned()
    };
    (!token.is_empty()).then_some(token)
}

fn read_request(stream: &TcpStream) -> Option<(String, String, Vec<u8>)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "POST" {
        return None;
    }
    let path = parts.next()?.split('?').next()?.to_owned();
    let (mut length, mut content_type) = (0usize, String::new());
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().ok()?;
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_owned();
        }
    }
    if length > 64 * 1024 {
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((path, content_type, body))
}

fn reply(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refresh_token_is_read_from_json_or_a_form() {
        assert_eq!(
            refresh_token(
                "application/json",
                br#"{"refresh_token":"rt-1","client_id":"c"}"#
            )
            .as_deref(),
            Some("rt-1")
        );
        assert_eq!(
            refresh_token(
                "application/x-www-form-urlencoded",
                b"grant_type=refresh_token&refresh_token=rt-2"
            )
            .as_deref(),
            Some("rt-2")
        );
        assert_eq!(refresh_token("application/json", b"{}"), None);
    }
}
