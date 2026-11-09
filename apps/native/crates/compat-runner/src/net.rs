//! The system HTTP stack for extension `fetch` and Store downloads. Starting a
//! request only enqueues it; a worker polls or waits, AppKit never does.
//!
//! The host process has no sandbox, so a Store extension's fetch is held to the
//! shipped connect policy (HTTPS only) and may not reach this Mac or its network.
//! Trusted extensions and agents keep full freedom. A hostname is resolved and
//! checked at the request and at every redirect; a name that changes its answer
//! between that check and NSURLSession's own lookup is not caught.
use pm_runtime_v8::{Cancellation, Error, FetchRequest, FetchResponse, HostRequest, Result};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use url::{Host, Url};

const MAX_REDIRECTS: usize = 8;

/// Who a request is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    /// Trusted code and the app's own downloads.
    Open,
    /// Confined code: public HTTPS hosts only, also across redirects.
    Public,
    /// `assets.importUrl`, as the shipped app downloads media: public HTTPS
    /// only, across at most five redirects, without credentials.
    Media,
    /// The app's reads of one service (the Store): that origin only, at the
    /// request and before every redirect hop, so a foreign hop is never
    /// contacted and a scheme downgrade is refused.
    Origin(&'static str),
}
/// Media downloads follow at most this many redirects.
const MEDIA_REDIRECTS: usize = 5;

fn local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
}
fn local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || first & 0xffc0 == 0xfec0
        || ip.to_ipv4_mapped().is_some_and(local_v4)
}
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => local_v4(ip),
        IpAddr::V6(ip) => local_v6(ip),
    }
}
/// Worker-only: a hostname is looked up here.
fn admit(url: &str, reach: Reach) -> Result<()> {
    if reach == Reach::Open {
        return Ok(());
    }
    if let Reach::Origin(origin) = reach {
        let origin = Url::parse(origin).map_err(|_| Error::Invalid("origin"))?;
        return match Url::parse(url) {
            Ok(url) if url.origin() == origin.origin() => Ok(()),
            _ => Err(Error::Host(format!(
                "A request to {} was sent elsewhere",
                origin.origin().ascii_serialization()
            ))),
        };
    }
    let refused = || Error::Host(format!("Extensions cannot reach {url}"));
    let parsed = Url::parse(url).map_err(|_| Error::Invalid("request URL"))?;
    if parsed.scheme() != "https" {
        return Err(refused());
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    match parsed.host().ok_or_else(refused)? {
        Host::Ipv4(ip) if local_v4(ip) => Err(refused()),
        Host::Ipv6(ip) if local_v6(ip) => Err(refused()),
        Host::Ipv4(_) | Host::Ipv6(_) => Ok(()),
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") {
                return Err(refused());
            }
            let found = (name.as_str(), port).to_socket_addrs();
            // An unresolvable name fails in NSURLSession with its own message.
            if found.is_ok_and(|mut found| found.any(|address| local(address.ip()))) {
                return Err(refused());
            }
            Ok(())
        }
    }
}

/// Credentials set for one origin never follow a redirect to another.
fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

type Slot = Arc<Mutex<Option<Result<FetchResponse>>>>;

#[cfg(target_os = "macos")]
mod system {
    use super::*;
    use block2::DynBlock;
    use objc2::{
        AnyThread, DefinedClass, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, ProtocolObject},
    };
    use objc2_foundation::{
        NSData, NSError, NSHTTPURLResponse, NSMutableCopying, NSMutableURLRequest,
        NSObjectProtocol, NSOperationQueue, NSString, NSTimeInterval, NSURL, NSURLRequest,
        NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURLSessionDataDelegate,
        NSURLSessionDataTask, NSURLSessionDelegate, NSURLSessionResponseDisposition,
        NSURLSessionTask, NSURLSessionTaskDelegate,
    };

    pub struct Task(Retained<NSURLSessionDataTask>);
    // SAFETY: NSURLSessionTask is documented thread-safe for resume and cancel.
    unsafe impl Send for Task {}
    impl Task {
        pub fn cancel(&self) {
            self.0.cancel();
        }
    }

    struct Delivery {
        limit: usize,
        reach: Reach,
        body: Mutex<Vec<u8>>,
        /// Why the task was cancelled from in here, ahead of its generic error.
        failure: Mutex<Option<Error>>,
        redirects: Mutex<usize>,
        fallback: String,
        result: Slot,
    }
    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "PMCompatFetchDelegate"]
        #[ivars = Delivery]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}
        unsafe impl NSURLSessionDelegate for Delegate {}
        unsafe impl NSURLSessionTaskDelegate for Delegate {
            #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
            unsafe fn redirected(
                &self,
                _: &NSURLSession,
                task: &NSURLSessionTask,
                _: &NSHTTPURLResponse,
                request: &NSURLRequest,
                done: &DynBlock<dyn Fn(*mut NSURLRequest)>,
            ) {
                let next = request.URL().and_then(|url| url.absoluteString());
                let next = next.map(|url| url.to_string()).unwrap_or_default();
                let delivery = self.ivars();
                let mut count = delivery.redirects.lock().unwrap();
                *count += 1;
                let limit = if delivery.reach == Reach::Media {
                    MEDIA_REDIRECTS
                } else {
                    MAX_REDIRECTS
                };
                let allowed = if *count > limit {
                    Err(Error::Host("Too many redirects".into()))
                } else {
                    admit(&next, delivery.reach)
                };
                drop(count);
                if let Err(error) = allowed {
                    *delivery.failure.lock().unwrap() = Some(error);
                    task.cancel();
                    done.call((std::ptr::null_mut(),));
                    return;
                }
                let previous = task
                    .currentRequest()
                    .and_then(|r| r.URL())
                    .and_then(|url| url.absoluteString())
                    .map(|url| url.to_string())
                    .unwrap_or_default();
                let next_request = request.mutableCopy();
                if !same_origin(&previous, &next) {
                    for name in ["Authorization", "Cookie", "Proxy-Authorization"] {
                        next_request.setValue_forHTTPHeaderField(None, &NSString::from_str(name));
                    }
                }
                done.call((Retained::as_ptr(&next_request) as *mut NSURLRequest,));
            }

            #[unsafe(method(URLSession:task:didCompleteWithError:))]
            fn completed(
                &self,
                session: &NSURLSession,
                task: &NSURLSessionTask,
                error: Option<&NSError>,
            ) {
                let delivery = self.ivars();
                let outcome = match (delivery.failure.lock().unwrap().take(), error) {
                    (Some(failure), _) => Err(failure),
                    (None, Some(error)) => {
                        Err(Error::Host(error.localizedDescription().to_string()))
                    }
                    (None, None) => {
                        let body = std::mem::take(&mut *delivery.body.lock().unwrap());
                        let response = task.response();
                        Ok(response_of(response.as_deref(), &delivery.fallback, body))
                    }
                };
                *delivery.result.lock().unwrap() = Some(outcome);
                session.finishTasksAndInvalidate();
            }
        }
        unsafe impl NSURLSessionDataDelegate for Delegate {
            #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
            unsafe fn responded(
                &self,
                _: &NSURLSession,
                task: &NSURLSessionDataTask,
                response: &NSURLResponse,
                done: &DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
            ) {
                // A declared size over the limit stops before any body arrives.
                let delivery = self.ivars();
                if response.expectedContentLength() > delivery.limit as i64 {
                    *delivery.failure.lock().unwrap() = Some(Error::Quota("response bytes"));
                    task.cancel();
                    done.call((NSURLSessionResponseDisposition::Cancel,));
                } else {
                    done.call((NSURLSessionResponseDisposition::Allow,));
                }
            }

            #[unsafe(method(URLSession:dataTask:didReceiveData:))]
            fn received(&self, _: &NSURLSession, task: &NSURLSessionDataTask, data: &NSData) {
                let delivery = self.ivars();
                let mut body = delivery.body.lock().unwrap();
                if body.len().saturating_add(data.len()) > delivery.limit {
                    body.clear();
                    *delivery.failure.lock().unwrap() = Some(Error::Quota("response bytes"));
                    task.cancel();
                } else {
                    body.extend_from_slice(&data.to_vec());
                }
            }
        }
    );

    fn response_of(
        response: Option<&NSURLResponse>,
        fallback: &str,
        body: Vec<u8>,
    ) -> FetchResponse {
        let http = response.and_then(|r| {
            let object: &AnyObject = r.as_ref();
            object.downcast_ref::<NSHTTPURLResponse>()
        });
        let Some(http) = http else {
            return FetchResponse {
                status: 200,
                url: fallback.to_owned(),
                headers: vec![],
                body,
            };
        };
        let fields = http.allHeaderFields();
        let mut headers = vec![];
        for key in fields.allKeys() {
            if let Some(value) = fields.objectForKey(&*key) {
                let name = key.downcast_ref::<NSString>().map(|s| s.to_string());
                let value = value.downcast_ref::<NSString>().map(|s| s.to_string());
                if let (Some(name), Some(value)) = (name, value) {
                    headers.push((name, value));
                }
            }
        }
        FetchResponse {
            status: http.statusCode() as u16,
            url: http
                .URL()
                .and_then(|u| u.absoluteString())
                .map_or(fallback.to_owned(), |u| u.to_string()),
            headers,
            body,
        }
    }

    /// One session per request: ephemeral, with nowhere to keep or find cookies,
    /// cached bodies or credentials. It is invalidated when its task completes.
    pub fn start(input: &FetchRequest, reach: Reach, result: Slot) -> Result<Task> {
        let url = NSURL::URLWithString(&NSString::from_str(&input.url))
            .ok_or(Error::Invalid("request URL"))?;
        let request = NSMutableURLRequest::requestWithURL(&url);
        request.setHTTPMethod(&NSString::from_str(&input.method));
        let timeout = input
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        request.setTimeoutInterval(timeout.as_secs_f64() as NSTimeInterval);
        for (name, value) in &input.headers {
            request.setValue_forHTTPHeaderField(
                Some(&NSString::from_str(value)),
                &NSString::from_str(name),
            );
        }
        if !input.body.is_empty() {
            request.setHTTPBody(Some(&NSData::with_bytes(&input.body)));
        }
        let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        configuration.setHTTPCookieStorage(None);
        configuration.setURLCredentialStorage(None);
        configuration.setURLCache(None);
        configuration.setHTTPShouldSetCookies(false);
        let delegate: Retained<Delegate> = {
            let this = Delegate::alloc().set_ivars(Delivery {
                limit: input.max_response_bytes,
                reach,
                body: Mutex::default(),
                failure: Mutex::default(),
                redirects: Mutex::default(),
                fallback: input.url.clone(),
                result,
            });
            // SAFETY: NSObject's designated initializer on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        };
        // SAFETY: the session retains its delegate until invalidated, and Delegate
        // only touches its own locked state, from the session's serial queue.
        let session = unsafe {
            NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                &configuration,
                Some(ProtocolObject::from_ref(&*delegate)),
                None::<&NSOperationQueue>,
            )
        };
        let task = session.dataTaskWithRequest(&request);
        task.resume();
        Ok(Task(task))
    }
}

struct Http {
    result: Slot,
    task: Arc<Mutex<Option<system::Task>>>,
    cancelled: Arc<AtomicBool>,
}
impl HostRequest for Http {
    fn poll(&mut self) -> Option<Result<FetchResponse>> {
        self.result.lock().unwrap().take()
    }
    fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(task) = &*self.task.lock().unwrap() {
            task.cancel();
        }
    }
}

/// Starts `request` and returns at once. The body stops at the caller's limit
/// while it arrives; it is never buffered past it.
pub fn fetch(request: FetchRequest, reach: Reach, _: Cancellation) -> Result<Box<dyn HostRequest>> {
    let http = Http {
        result: Arc::default(),
        task: Arc::default(),
        cancelled: Arc::default(),
    };
    let (result, task, cancelled) = (
        http.result.clone(),
        http.task.clone(),
        http.cancelled.clone(),
    );
    // Name resolution blocks, and the caller may be serving other requests.
    std::thread::spawn(move || {
        let started = admit(&request.url, reach)
            .and_then(|()| system::start(&request, reach, result.clone()));
        match started {
            Ok(started) => {
                let mut slot = task.lock().unwrap();
                if cancelled.load(Ordering::Acquire) {
                    started.cancel();
                }
                *slot = Some(started);
            }
            Err(error) => *result.lock().unwrap() = Some(Err(error)),
        }
    });
    Ok(Box::new(http))
}

/// Worker-only: the whole response, or the first failure.
pub fn get(url: &str, max_bytes: usize, seconds: u64) -> Result<FetchResponse> {
    request(url, max_bytes, seconds, Reach::Open)
}
/// Worker-only: a read that may not leave `origin`, even by redirect.
pub fn get_within(
    origin: &'static str,
    url: &str,
    max_bytes: usize,
    seconds: u64,
) -> Result<FetchResponse> {
    request(url, max_bytes, seconds, Reach::Origin(origin))
}
/// Worker-only: `assets.importUrl`'s download (`Reach::Media`).
pub fn download(url: &str, max_bytes: usize, seconds: u64) -> Result<FetchResponse> {
    request(url, max_bytes, seconds, Reach::Media)
}
fn request(url: &str, max_bytes: usize, seconds: u64, reach: Reach) -> Result<FetchResponse> {
    send(
        FetchRequest {
            url: url.to_owned(),
            method: "GET".into(),
            headers: vec![],
            body: vec![],
            max_response_bytes: max_bytes,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(seconds),
        },
        reach,
    )
}
/// Worker-only: any method, headers and body, waited for until its deadline.
pub fn send(request: FetchRequest, reach: Reach) -> Result<FetchResponse> {
    let deadline = request.deadline;
    let mut request = fetch(request, reach, Cancellation::default())?;
    loop {
        if let Some(result) = request.poll() {
            return result;
        }
        if std::time::Instant::now() >= deadline {
            request.cancel();
            return Err(Error::Host("Request timed out".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn serve(replies: Vec<Vec<u8>>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen = vec![];
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 2048];
                let read = stream.read(&mut buffer).unwrap_or(0);
                seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let _ = stream.write_all(&reply);
            }
            seen
        });
        (port, server)
    }
    #[test]
    fn the_system_stack_returns_status_headers_and_body_and_enforces_the_limit() {
        let reply = b"HTTP/1.1 201 Created\r\nX-Synthetic: yes\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec();
        let (port, server) = serve(vec![reply.clone(), reply]);
        let url = format!("http://127.0.0.1:{port}/synthetic");
        let reply = get(&url, 1024, 10).unwrap();
        assert_eq!((reply.status, &reply.body[..]), (201, &b"hello"[..]));
        assert!(
            reply
                .headers
                .iter()
                .any(|(k, v)| k == "X-Synthetic" && v == "yes")
        );
        assert!(matches!(
            get(&url, 2, 10),
            Err(Error::Quota("response bytes"))
        ));
        server.join().unwrap();
    }
    #[test]
    fn a_body_without_a_declared_size_stops_at_the_limit_while_it_arrives() {
        let mut reply =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for _ in 0..64 {
            reply.extend_from_slice(b"1000\r\n");
            reply.extend_from_slice(&[b'x'; 4096]);
            reply.extend_from_slice(b"\r\n");
        }
        reply.extend_from_slice(b"0\r\n\r\n");
        let (port, server) = serve(vec![reply]);
        let url = format!("http://127.0.0.1:{port}/chunked");
        assert!(matches!(
            get(&url, 8192, 10),
            Err(Error::Quota("response bytes"))
        ));
        server.join().unwrap();
    }
    #[test]
    fn a_redirect_drops_credentials_across_origins() {
        let (target, target_server) = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec(),
        ]);
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (port, server) = serve(vec![redirect.into_bytes()]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut request = fetch(
            FetchRequest {
                url: format!("http://127.0.0.1:{port}/start"),
                method: "GET".into(),
                headers: vec![("Authorization".into(), "Bearer synthetic".into())],
                body: vec![],
                max_response_bytes: 1024,
                deadline,
            },
            Reach::Open,
            Cancellation::default(),
        )
        .unwrap();
        let reply = loop {
            if let Some(result) = request.poll() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(reply.body, b"ok");
        assert!(server.join().unwrap()[0].contains("Bearer synthetic"));
        assert!(!target_server.join().unwrap()[0].contains("Bearer synthetic"));
    }
    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }
    #[test]
    fn origin_reads_follow_redirects_only_within_the_origin() {
        // A hop to another origin is refused before it is contacted.
        let (foreign, foreign_server) = serve(vec![]);
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{foreign}/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (port, server) = serve(vec![redirect.into_bytes()]);
        let origin = leak(format!("http://127.0.0.1:{port}"));
        let refused = get_within(origin, &format!("{origin}/v1/store/a"), 1024, 10);
        assert!(
            matches!(&refused, Err(Error::Host(m)) if m.contains("sent elsewhere")),
            "{refused:?}"
        );
        server.join().unwrap();
        drop(foreign_server);
        // A chain that stays on the origin is followed.
        let (port, server) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /v1/store/b\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec(),
        ]);
        let origin = leak(format!("http://127.0.0.1:{port}"));
        let reply = get_within(origin, &format!("{origin}/v1/store/a"), 1024, 10).unwrap();
        assert_eq!(reply.body, b"ok");
        server.join().unwrap();
        // A start elsewhere, or the same host on another scheme or port, is refused.
        let store = "https://store.example";
        for url in [
            "http://store.example/v1/store/a",
            "https://store.example:8443/v1/store/a",
            "https://other.example/v1/store/a",
            "https://store.example.evil/v1/store/a",
        ] {
            assert!(admit(url, Reach::Origin(store)).is_err(), "{url}");
        }
        assert!(admit("https://store.example/v1/store/a", Reach::Origin(store)).is_ok());
    }
    #[test]
    fn confined_requests_stay_on_public_https() {
        for url in [
            "http://example.com/",
            "file:///etc/hosts",
            "https://localhost/",
            "https://printer.local/",
            "https://127.0.0.1:8443/",
            "https://[::1]/",
            "https://[::ffff:10.0.0.1]/",
            "https://10.0.0.1/",
            "https://172.16.0.1/",
            "https://192.168.1.1/",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/",
            "https://[fd00::1]/",
            "https://[fe80::1]/",
        ] {
            assert!(admit(url, Reach::Public).is_err(), "{url}");
        }
        assert!(admit("https://93.184.216.34/", Reach::Public).is_ok());
        assert!(admit("http://127.0.0.1/", Reach::Open).is_ok());
    }
}
