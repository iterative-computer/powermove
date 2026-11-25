//! Just enough HTTP/1.1 over std sockets for a loopback model endpoint: read a
//! request (content-length or chunked body), write a JSON or SSE response,
//! and a plain-HTTP client for the recording proxy's upstream. No TLS.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

pub const MAX_BODY: usize = 64 << 20;

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    /// Lowercased names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn read_line(r: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let n = r.read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn read_headers(r: &mut impl BufRead) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();
    loop {
        let line = read_line(r)?;
        if line.is_empty() {
            return Ok(headers);
        }
        if headers.len() > 200 {
            return Err("too many headers".into());
        }
        let (k, v) = line.split_once(':').ok_or("malformed header")?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
    }
}

fn read_body(r: &mut impl BufRead, headers: &[(String, String)]) -> Result<Vec<u8>, String> {
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.as_str())
    };
    if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        let mut body = Vec::new();
        loop {
            let size = read_line(r)?;
            let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| "malformed chunk size")?;
            if size == 0 {
                // trailers
                while !read_line(r)?.is_empty() {}
                return Ok(body);
            }
            if body.len() + size > MAX_BODY {
                return Err("body too large".into());
            }
            let at = body.len();
            body.resize(at + size, 0);
            r.read_exact(&mut body[at..]).map_err(|e| e.to_string())?;
            read_line(r)?;
        }
    }
    let len: usize = get("content-length")
        .map(|v| v.parse().map_err(|_| "malformed content-length"))
        .transpose()?
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err("body too large".into());
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

pub fn read_request(stream: &TcpStream) -> Result<HttpRequest, String> {
    let mut r = BufReader::new(stream);
    let line = read_line(&mut r)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(format!("malformed request line: {line}"));
    };
    let headers = read_headers(&mut r)?;
    let body = read_body(&mut r, &headers)?;
    Ok(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body,
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

/// A complete JSON response, then the connection closes.
pub fn write_json(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reason(status),
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// Start an event stream; the body ends when the connection closes.
pub fn start_sse(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
        reason(status)
    )?;
    stream.flush()
}

/// A plain-HTTP response read by the client side.
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    reader: BufReader<TcpStream>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn is_sse(&self) -> bool {
        self.header("content-type")
            .is_some_and(|c| c.starts_with("text/event-stream"))
    }
    /// Read the body in pieces as they arrive (chunked, sized or until close).
    pub fn for_each_piece(
        mut self,
        mut f: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let chunked = self
            .header("transfer-encoding")
            .is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
        if chunked {
            loop {
                let size = read_line(&mut self.reader)?;
                let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16)
                    .map_err(|_| "malformed chunk size")?;
                if size == 0 {
                    return Ok(());
                }
                let mut buf = vec![0; size];
                self.reader
                    .read_exact(&mut buf)
                    .map_err(|e| e.to_string())?;
                read_line(&mut self.reader)?;
                f(&buf)?;
            }
        }
        let mut left = self
            .header("content-length")
            .and_then(|v| v.parse::<usize>().ok());
        let mut buf = [0u8; 16 * 1024];
        loop {
            let want = left.map_or(buf.len(), |l| l.min(buf.len()));
            if want == 0 {
                return Ok(());
            }
            let n = self
                .reader
                .read(&mut buf[..want])
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            if let Some(l) = left.as_mut() {
                *l -= n;
            }
            f(&buf[..n])?;
        }
    }
    pub fn body(self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.for_each_piece(|p| {
            out.extend_from_slice(p);
            Ok(())
        })?;
        Ok(out)
    }
}

/// Send one request to `http://host:port` and return the response head.
pub fn send(
    origin: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<HttpResponse, String> {
    let host = origin.strip_prefix("http://").ok_or_else(|| {
        format!("{origin}: only plain-HTTP origins are supported (no TLS in this crate)")
    })?;
    let host = host.trim_end_matches('/');
    let mut stream = TcpStream::connect(host).map_err(|e| format!("{host}: {e}"))?;
    // A stalled upstream fails the exchange instead of hanging the caller.
    let deadline = Some(std::time::Duration::from_secs(120));
    stream
        .set_read_timeout(deadline)
        .and_then(|()| stream.set_write_timeout(deadline))
        .map_err(|e| e.to_string())?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n");
    for (k, v) in headers {
        if !matches!(
            k.as_str(),
            "host" | "connection" | "content-length" | "transfer-encoding"
        ) {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    head.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let status_line = read_line(&mut reader)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;
    let headers = read_headers(&mut reader)?;
    Ok(HttpResponse {
        status,
        headers,
        reader,
    })
}
