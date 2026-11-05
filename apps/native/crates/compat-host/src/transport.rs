use crate::*;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_PENDING_BYTES: usize = 8 * 1024 * 1024;
/// Redacted Debug intentionally omits the per-generation capability.
pub struct Channel {
    token: String,
    pub generation: Generation,
    next: u64,
    ack: u64,
    bytes: usize,
    queue: VecDeque<(u64, Value, usize)>,
}
impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl Channel {
    pub fn new(token: String, generation: Generation) -> Self {
        Self {
            token,
            generation,
            next: 1,
            ack: 0,
            bytes: 0,
            queue: VecDeque::new(),
        }
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn base(&self) -> String {
        format!("pmcompat://host/{}/", self.token)
    }
    pub fn route(&self, raw: &str) -> Result<(String, Option<String>)> {
        if raw.len() > MAX_MESSAGE_BYTES * 3 {
            return Err(Error::InvalidRequest);
        }
        let u = url::Url::parse(raw).map_err(|_| Error::InvalidRequest)?;
        if u.scheme() != "pmcompat"
            || u.host_str() != Some("host")
            || u.port().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
        {
            return Err(Error::Forbidden);
        }
        let mut parts = u.path().split('/').skip(1);
        if parts.next() != Some(self.token.as_str()) {
            return Err(Error::Forbidden);
        }
        let route = parts.next().ok_or(Error::InvalidRequest)?.to_owned();
        if parts.next().is_some() || !matches!(route.as_str(), "call" | "events" | "page") {
            return Err(Error::InvalidRequest);
        }
        let pairs: Vec<_> = u.query_pairs().collect();
        if pairs.len() > 1 || pairs.first().is_some_and(|(k, _)| k != "q") {
            return Err(Error::InvalidRequest);
        }
        let payload = pairs.first().map(|(_, v)| v.to_string());
        if payload
            .as_ref()
            .is_some_and(|p| p.len() > MAX_MESSAGE_BYTES)
        {
            return Err(Error::InvalidRequest);
        }
        Ok((route, payload))
    }
    pub fn send(&mut self, generation: Generation, event: Value) -> Result<()> {
        if generation != self.generation {
            return Err(Error::StaleGeneration);
        }
        let size = serde_json::to_vec(&event)
            .map_err(|_| Error::InvalidRequest)?
            .len();
        if size > MAX_MESSAGE_BYTES
            || self.queue.len() >= 256
            || self.bytes + size > MAX_PENDING_BYTES
        {
            return Err(Error::Backpressure);
        }
        self.queue.push_back((self.next, event, size));
        self.next += 1;
        self.bytes += size;
        Ok(())
    }
    /// A poll acknowledges the last batch only after the applier has consumed it.
    /// Duplicate polls replay the unacknowledged tail; no silent eviction.
    pub fn poll(&mut self, after: u64) -> Result<Option<Value>> {
        if after < self.ack || after >= self.next {
            return Err(Error::InvalidRequest);
        }
        self.ack = after;
        while self.queue.front().is_some_and(|(s, _, _)| *s <= after) {
            self.bytes -= self.queue.pop_front().unwrap().2;
        }
        if self.queue.is_empty() {
            return Ok(None);
        }
        // Bound each response too. A slow page sees backpressure at send().
        let mut used = 0;
        let mut events = vec![];
        let mut seq = after;
        for (s, event, size) in &self.queue {
            if used + size > MAX_MESSAGE_BYTES && !events.is_empty() {
                break;
            }
            used += size;
            seq = *s;
            events.push(event.clone());
        }
        Ok(Some(
            json!({"generation":self.generation,"seq":seq,"events":events}),
        ))
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn csp(nonce: &str, trust: &Trust) -> String {
    // Rendering assets retain the manifest's network permission. JS fetch and
    // WebSocket live in V8 and are permission-enforced by the kernel; this page
    // may connect only to its authenticated host channel and local data/blobs.
    let remote = match trust {
        Trust::Trusted => " http: https:",
        Trust::Store { .. } if trust.permits("network") => " https:",
        _ => "",
    };
    format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline' data: blob:{remote}; img-src data: blob:{remote}; font-src data: blob:{remote}; connect-src pmcompat: data: blob:; media-src data: blob:{remote}; object-src 'none'; worker-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'"
    )
}
#[cfg(test)]
pub(crate) fn page(base: &str, nonce: &str, generation: Generation) -> String {
    page_with_applier(base, nonce, generation, PAGE_APPLIER, &Trust::Trusted)
}
#[cfg(any(target_os = "macos", test))]
pub(crate) fn page_with_applier(
    base: &str,
    nonce: &str,
    generation: Generation,
    applier: &str,
    trust: &Trust,
) -> String {
    let cfg = json!({"base":base,"generation":generation});
    format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{}\"><meta name=\"color-scheme\" content=\"dark light\"><style>html,body{{margin:0;background:transparent;font:14px -apple-system;color:CanvasText}}[data-pm-panel]{{position:fixed;overflow:visible}}[data-pm-panel][hidden]{{display:none}}</style><script nonce=\"{}\">const PM_HOST_CONFIG={};{}</script>",
        csp(nonce, trust),
        nonce,
        cfg,
        applier
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn channel() -> Channel {
        Channel::new("test-capability".into(), 7)
    }
    #[test]
    fn routing_rejects_wrong_authority_and_token() {
        let c = channel();
        for u in [
            "http://host/test-capability/call?q=x",
            "pmcompat://evil/test-capability/call?q=x",
            "pmcompat://host/other/call?q=x",
            "pmcompat://user@host/test-capability/call?q=x",
            "pmcompat://host/test-capability/call?q=x&q=y",
            "pmcompat://host/test-capability/call?q=x#secret",
            "pmcompat://host/test-capability/../call?q=x",
        ] {
            assert!(c.route(u).is_err(), "{u}");
        }
    }
    #[test]
    fn query_decodes_without_double_decode() {
        assert_eq!(
            channel()
                .route("pmcompat://host/test-capability/call?q=%7B%22v%22%3A%22%2520%22%7D")
                .unwrap(),
            ("call".into(), Some("{\"v\":\"%20\"}".into()))
        );
    }
    #[test]
    fn poll_replays_until_acknowledged() {
        let mut c = channel();
        c.send(7, json!({"n":1})).unwrap();
        c.send(7, json!({"n":2})).unwrap();
        let a = c.poll(0).unwrap().unwrap();
        assert_eq!(a, c.poll(0).unwrap().unwrap());
        assert_eq!(a["seq"], 2);
        assert_eq!(a["events"].as_array().unwrap().len(), 2);
        assert!(c.poll(2).unwrap().is_none());
        assert!(c.poll(1).is_err());
        assert!(c.poll(3).is_err());
    }
    #[test]
    fn stale_generation_never_enters_queue() {
        let mut c = channel();
        assert_eq!(c.send(6, json!({})), Err(Error::StaleGeneration));
        assert!(c.poll(0).unwrap().is_none());
    }
    #[test]
    fn bounded_queue_never_silently_drops() {
        let mut c = channel();
        for n in 0..256 {
            c.send(7, json!({"n":n})).unwrap()
        }
        assert_eq!(c.send(7, json!({"n":257})), Err(Error::Backpressure));
        let a = c.poll(0).unwrap().unwrap();
        assert_eq!(a["events"][0]["n"], 0);
        assert_eq!(a["seq"], 256);
    }
    #[test]
    fn oversized_messages_fail_before_admission() {
        let mut c = channel();
        assert_eq!(
            c.send(7, json!("x".repeat(MAX_MESSAGE_BYTES))),
            Err(Error::Backpressure)
        );
        assert!(c.poll(0).unwrap().is_none());
    }
    #[test]
    fn custom_adapter_replaces_fixture_code() {
        let html = page_with_applier(
            "pmcompat://host/fixture/",
            "fixture",
            1,
            "window.customAdapter=true;",
            &Trust::Trusted,
        );
        assert!(html.contains("window.customAdapter=true;"));
        assert!(!html.contains("Minimal adapter fixture"));
    }
    #[test]
    fn manifest_network_permission_changes_asset_csp_only() {
        let denied = Trust::Store {
            extension: "fixture".into(),
            permissions: Default::default(),
        };
        let allowed = Trust::Store {
            extension: "fixture".into(),
            permissions: ["network".into()].into(),
        };
        assert!(!csp("fixture", &denied).contains("https:"));
        let granted = csp("fixture", &allowed);
        assert!(granted.contains("img-src data: blob: https:"));
        assert!(granted.contains("connect-src pmcompat: data: blob:;"));
        assert!(!granted.contains(" http:"));
    }
    #[test]
    fn debug_redacts_capability() {
        assert!(!format!("{:?}", channel()).contains("test-capability"));
    }
    #[test]
    fn bootstrap_has_no_eval_or_network_origin() {
        let h = page("pmcompat://host/test/", "test-nonce", 1);
        assert!(h.contains("script-src 'nonce-test-nonce'"));
        assert!(h.contains("connect-src pmcompat:"));
        assert!(!PAGE_APPLIER.contains("evaluateJavaScript"));
        assert!(!PAGE_APPLIER.contains("eval("));
    }
}
