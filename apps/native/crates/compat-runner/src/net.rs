//! The system HTTP stack for extension `fetch` and Store downloads. Starting a
//! request only enqueues it; a worker polls or waits, AppKit never does.
use pm_runtime_v8::{Cancellation, Error, FetchRequest, FetchResponse, HostRequest, Result};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
mod system {
    use super::*;
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::AnyObject};
    use objc2_foundation::{
        NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSTimeInterval, NSURL,
        NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURLSessionDataTask,
    };
    pub struct Task(Retained<NSURLSessionDataTask>);
    // SAFETY: NSURLSessionTask is documented thread-safe for resume and cancel.
    unsafe impl Send for Task {}
    impl Task {
        pub fn cancel(&self) {
            self.0.cancel();
        }
    }
    fn session() -> Retained<NSURLSession> {
        // Ephemeral: no shared cookies, cache or credentials across extensions.
        let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        NSURLSession::sessionWithConfiguration(&configuration)
    }
    pub fn start(
        input: &FetchRequest,
        result: Arc<Mutex<Option<Result<FetchResponse>>>>,
    ) -> Result<Task> {
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
        let limit = input.max_response_bytes;
        let fallback = input.url.clone();
        let done = RcBlock::new(
            move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
                // SAFETY: the session owns these for the duration of this callback.
                let outcome = unsafe {
                    if let Some(error) = error.as_ref() {
                        Err(Error::Host(error.localizedDescription().to_string()))
                    } else {
                        let body = data.as_ref().map(|d| d.to_vec()).unwrap_or_default();
                        if body.len() > limit {
                            Err(Error::Quota("response bytes"))
                        } else {
                            let http = response
                                .as_ref()
                                .and_then(|r| {
                                    let object: &AnyObject = r.as_ref();
                                    object.downcast_ref::<NSHTTPURLResponse>()
                                });
                            let (status, url, headers) = http.map_or(
                                (200, fallback.clone(), vec![]),
                                |http| {
                                    let headers = http.allHeaderFields();
                                    let mut list = vec![];
                                    for key in headers.allKeys() {
                                        if let Some(value) = headers.objectForKey(&*key) {
                                            let name = key
                                                .downcast_ref::<NSString>()
                                                .map(|s| s.to_string());
                                            let value = value
                                                .downcast_ref::<NSString>()
                                                .map(|s| s.to_string());
                                            if let (Some(name), Some(value)) = (name, value) {
                                                list.push((name, value));
                                            }
                                        }
                                    }
                                    (
                                        http.statusCode() as u16,
                                        http.URL()
                                            .and_then(|u| u.absoluteString())
                                            .map_or(fallback.clone(), |u| u.to_string()),
                                        list,
                                    )
                                },
                            );
                            Ok(FetchResponse {
                                status,
                                url,
                                headers,
                                body,
                            })
                        }
                    }
                };
                *result.lock().unwrap() = Some(outcome);
            },
        );
        // SAFETY: the block is an owned, thread-safe completion the session retains.
        let task = unsafe { session().dataTaskWithRequest_completionHandler(&request, &done) };
        task.resume();
        Ok(Task(task))
    }
}

struct Http {
    result: Arc<Mutex<Option<Result<FetchResponse>>>>,
    task: system::Task,
}
impl HostRequest for Http {
    fn poll(&mut self) -> Option<Result<FetchResponse>> {
        self.result.lock().unwrap().take()
    }
    fn cancel(&mut self) {
        self.task.cancel();
    }
}

/// Starts `request` and returns at once. The caller's limits apply to the body.
pub fn fetch(request: FetchRequest, _: Cancellation) -> Result<Box<dyn HostRequest>> {
    let result: Arc<Mutex<Option<Result<FetchResponse>>>> = Arc::default();
    let task = system::start(&request, result.clone())?;
    Ok(Box::new(Http { result, task }))
}

/// Worker-only: the whole response, or the first failure.
pub fn get(url: &str, max_bytes: usize, seconds: u64) -> Result<FetchResponse> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut request = fetch(
        FetchRequest {
            url: url.to_owned(),
            method: "GET".into(),
            headers: vec![],
            body: vec![],
            max_response_bytes: max_bytes,
            deadline,
        },
        Cancellation::default(),
    )?;
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
    #[test]
    fn the_system_stack_returns_status_headers_and_body_and_enforces_the_limit() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 1024];
                let _ = stream.read(&mut buffer);
                stream
                    .write_all(b"HTTP/1.1 201 Created\r\nX-Synthetic: yes\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
                    .unwrap();
            }
        });
        let url = format!("http://127.0.0.1:{port}/synthetic");
        let reply = get(&url, 1024, 10).unwrap();
        assert_eq!((reply.status, &reply.body[..]), (201, &b"hello"[..]));
        assert!(reply.headers.iter().any(|(k, v)| k == "X-Synthetic" && v == "yes"));
        assert!(matches!(get(&url, 2, 10), Err(Error::Quota("response bytes"))));
        server.join().unwrap();
    }
}
