//! HTTP + WebSocket front door on loopback.
//!
//! Serves the renderer bundle, user extension bundles, and the one WebSocket
//! (`/__powermove/ws`) that `host/web-bridge.ts` speaks the wire protocol over.
//! The window presents the token once in the URL and gets an HttpOnly cookie.

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router as Http;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;

use crate::host::Host;
use crate::router::{Call, Client};
use crate::wire::{self, ClientMessage};

const COOKIE: &str = "pm_session";
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// The editor document policy from `src/main/security-policy.ts`.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data: http: https:; media-src 'self' blob:; font-src 'self'; connect-src 'self' blob: http: https: ws: wss:; worker-src 'self' blob:; frame-src 'self' about: blob: app:";
/// Generated scripts keep eval only inside the opaque, no-network sandbox.
const SANDBOX_CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-eval'; worker-src blob:; connect-src 'none'";

pub async fn bind(host: Arc<Host>) -> anyhow::Result<(SocketAddr, impl Future<Output = ()>)> {
    // The page's storage (media, recovery) belongs to its origin, port included,
    // so keep the configured port when it is free and move only when it is not.
    let listener = match TcpListener::bind(("127.0.0.1", host.config.port)).await {
        Ok(listener) => listener,
        Err(error) if host.config.port != 0 => {
            tracing::warn!("port {} is unavailable ({error}); using another", host.config.port);
            TcpListener::bind(("127.0.0.1", 0)).await?
        }
        Err(error) => return Err(error.into()),
    };
    let address = listener.local_addr()?;
    let app = Http::new()
        .route("/__powermove/ws", get(socket))
        .fallback(get(files))
        .with_state(host);
    let serve = async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!("host server stopped: {error}");
        }
    };
    Ok((address, serve))
}

fn authorized(host: &Host, headers: &HeaderMap) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .any(|(name, value)| name == COOKIE && constant_time_eq(value.as_bytes(), host.token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn text(status: StatusCode, body: &'static str) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

#[derive(Deserialize)]
struct SocketQuery {
    project: Option<String>,
}

async fn socket(State(host): State<Arc<Host>>, headers: HeaderMap, Query(query): Query<SocketQuery>, upgrade: WebSocketUpgrade) -> Response {
    if !authorized(&host, &headers) {
        return text(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let project = query.project.filter(|id| crate::ids::is_project_id(id));
    upgrade.max_message_size(MAX_FRAME_BYTES).on_upgrade(move |socket| run_socket(host, socket, project))
}

async fn run_socket(host: Arc<Host>, socket: WebSocket, project: Option<String>) {
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut queue) = mpsc::unbounded_channel::<Vec<u8>>();
    let client = host.connect(outbox, project);
    tracing::info!("window {} connected", client.id);
    let writer = tokio::spawn(async move {
        while let Some(frame) = queue.recv().await {
            if sink.send(Message::Binary(frame.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Binary(data) => dispatch(&host, &client, data),
            Message::Close(_) => break,
            _ => {}
        }
    }
    client.close();
    writer.abort();
    tracing::info!("window {} disconnected", client.id);
}

/// Decode one frame and run its handler on its own task so slow requests
/// (an ffmpeg conversion) never block the socket.
fn dispatch(host: &Arc<Host>, client: &Arc<Client>, data: bytes::Bytes) {
    let payload = match wire::decode(data) {
        Ok(payload) => payload,
        Err(error) => return tracing::warn!("window {}: {error}", client.id),
    };
    let message = match ClientMessage::parse(payload.json) {
        Ok(message) => message,
        Err(error) => return tracing::warn!("window {}: {error}", client.id),
    };
    let (id, channel, args) = match message {
        ClientMessage::Answer { id, value } => return client.answer(id, value),
        ClientMessage::Invoke { id, channel, args } | ClientMessage::Sync { id, channel, args } => (Some(id), channel, args),
        ClientMessage::Send { channel, args } => (None, channel, args),
    };
    let Some(handler) = host.router().get(&channel) else {
        tracing::warn!("unhandled channel {channel}");
        if let Some(id) = id {
            client.push(wire::result_err(id, &format!("No handler registered for '{channel}'")));
        }
        return;
    };
    let call = Call { host: host.clone(), client: client.clone(), args, blobs: Arc::new(payload.blobs) };
    let client = client.clone();
    tokio::spawn(async move {
        let result = handler(call).await;
        match (id, result) {
            (Some(id), Ok(value)) => client.push(wire::result_ok(id, value)),
            (Some(id), Err(error)) => client.push(wire::result_err(id, &format!("{error:#}"))),
            (None, Err(error)) => tracing::warn!("{channel}: {error:#}"),
            (None, Ok(_)) => {}
        }
    });
}

#[derive(Deserialize)]
struct FileQuery {
    token: Option<String>,
}

async fn files(State(host): State<Arc<Host>>, uri: Uri, headers: HeaderMap, Query(query): Query<FileQuery>) -> Response {
    if let Some(token) = query.token {
        if !constant_time_eq(token.as_bytes(), host.token.as_bytes()) {
            return text(StatusCode::FORBIDDEN, "Wrong token");
        }
        let mut response = Redirect::to(uri.path()).into_response();
        let cookie = format!("{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000", host.token);
        response.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
        return response;
    }
    if !authorized(&host, &headers) {
        return text(StatusCode::UNAUTHORIZED, "Open Powermove from its app window.");
    }
    let path = percent_decode(uri.path());
    if let Some(rest) = path.strip_prefix("/ext/") {
        return extension_bundle(&host, rest).await;
    }
    let relative = path.trim_start_matches('/');
    let relative = if relative.is_empty() { "index.html" } else { relative };
    let Some(file) = contained(&host.config.renderer_dir, relative) else { return text(StatusCode::FORBIDDEN, "Forbidden") };
    let mut response = match stream_file(&file).await {
        Some(response) => response,
        None => return text(StatusCode::NOT_FOUND, "Not found"),
    };
    let headers = response.headers_mut();
    let csp = if relative == "host/sandbox.html" { SANDBOX_CSP } else { CONTENT_SECURITY_POLICY };
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if relative.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" }),
    );
    if (relative.starts_with("host/") && relative.ends_with(".js")) || is_font(relative) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    }
    response
}

/// `/ext/<id>/bundle.js`: a compiled user extension.
async fn extension_bundle(host: &Host, rest: &str) -> Response {
    let Some((id, "bundle.js")) = rest.split_once('/') else { return text(StatusCode::NOT_FOUND, "Not found") };
    if !crate::ids::is_extension_id(id) {
        return text(StatusCode::NOT_FOUND, "Not found");
    }
    let file = host.config.user_data.join("extensions-build").join(id).join("bundle.js");
    match stream_file(&file).await {
        Some(mut response) => {
            response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        None => text(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn stream_file(file: &Path) -> Option<Response> {
    let handle = tokio::fs::File::open(file).await.ok()?;
    let metadata = handle.metadata().await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut response = Body::from_stream(ReaderStream::new(handle)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime_type(file)));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(metadata.len()));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Some(response)
}

/// Resolve `relative` under `root`, refusing anything that climbs out.
pub fn contained(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(path)
}

fn is_font(path: &str) -> bool {
    [".woff", ".woff2", ".ttf", ".otf"].iter().any(|extension| path.to_ascii_lowercase().ends_with(extension))
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&path[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Served with nosniff, so anything unlisted is rejected by media elements
/// rather than sniffed (mirrors `src/main/mime.ts`).
pub fn mime_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "aac" => "audio/aac",
        "avif" => "image/avif",
        "css" => "text/css; charset=utf-8",
        "flac" => "audio/flac",
        "gif" => "image/gif",
        "html" => "text/html; charset=utf-8",
        "ico" => "image/x-icon",
        "jpeg" | "jpg" => "image/jpeg",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "m4a" => "audio/mp4",
        "m4v" | "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "otf" => "font/otf",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "wav" => "audio/wav",
        "webm" => "video/webm",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
