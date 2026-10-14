//! Media conversion and export encoding with ffmpeg (port of
//! `src/main/media-proxy.ts` and `src/main/render-encoder.ts`).
//!
//! Every proxy and render job belongs to the window that started it. Closing
//! the window stops its ffmpeg processes and deletes their temp directories.

mod convert;
mod process;
mod proxy;
mod render;
mod sequence;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::host::Config;
use crate::router::{Call, Client, Router};
use crate::wire::Payload;
use convert::Converters;
use proxy::{ProxyService, is_token};
use render::{EncoderOptions, RenderEncoder};

#[derive(Clone)]
struct Media(Arc<Inner>);

struct Inner {
    proxies: ProxyService,
    render: RenderEncoder,
    /// Cancelled when the window closes; stops its running conversions.
    windows: Mutex<HashMap<u64, CancellationToken>>,
}

impl Media {
    /// The window's cancellation token, installing its cleanup on first use.
    fn window(&self, client: &Arc<Client>) -> CancellationToken {
        let mut windows = self.0.windows.lock();
        if let Some(token) = windows.get(&client.id) {
            return token.clone();
        }
        let token = CancellationToken::new();
        windows.insert(client.id, token.clone());
        drop(windows);
        let (inner, owner, cancel) = (self.0.clone(), client.id, token.clone());
        let runtime = tokio::runtime::Handle::current();
        client.on_close(move || {
            cancel.cancel();
            runtime.spawn(async move {
                inner.windows.lock().remove(&owner);
                inner.render.release_owner(owner).await;
                inner.proxies.release_owner(owner).await;
            });
        });
        token
    }
}

pub async fn register(router: &mut Router, config: &Config) -> Result<()> {
    let convert = Converters { ffmpeg: config.ffmpeg.clone() };
    let media = Media(Arc::new(Inner {
        proxies: ProxyService::new(config.temp_dir.clone(), convert),
        render: RenderEncoder::new(config.ffmpeg.clone(), config.temp_dir.clone()),
        windows: Mutex::default(),
    }));

    router.json_with(&media, "media-preview:begin", |media, call| async move {
        let total = call.arg(0).as_u64().context("Invalid preview source size")?;
        media.window(&call.client);
        Ok(json!(media.0.proxies.begin_preview(total, call.client.id).await?))
    });
    router.json_with(&media, "media-preview:chunk", |media, call| async move {
        let invalid = || anyhow!("Invalid preview source chunk");
        let request = object(&call).ok_or_else(invalid)?;
        let token = request.get("token").and_then(Value::as_str).ok_or_else(invalid)?;
        let offset = request.get("offset").and_then(Value::as_u64).ok_or_else(invalid)?;
        let data = call.bytes(request.get("data").unwrap_or(&Value::Null)).map_err(|_| invalid())?;
        media.0.proxies.write_preview(token, call.client.id, offset, &data).await?;
        Ok(Value::Null)
    });
    router.json_with(&media, "media-preview:finish", |media, call| async move {
        let token = call.arg(0).as_str().context("Incomplete preview source")?;
        let cancel = media.window(&call.client);
        let size = media.0.proxies.finish_preview(token, call.client.id, &cancel).await?;
        Ok(json!({ "token": token, "size": size }))
    });

    router.json_with(&media, "media-proxy:create", |media, call| async move {
        let source_path = source_request(&call, "media-proxy:create")?;
        let cancel = media.window(&call.client);
        let result = media.0.proxies.create(&source_path, call.client.id, &cancel).await;
        Ok(proxy_result(result, "video/webm", Some("Could not optimize this video for playback")))
    });

    router.json_with(&media, "media-sequence:create", |media, call| async move {
        const CHANNEL: &str = "media-sequence:create";
        let request = object(&call).ok_or_else(|| validation(CHANNEL, "expected an object"))?;
        let request_id = request_id(request, CHANNEL)?;
        let source_paths = request.get("sourcePaths").and_then(Value::as_array).and_then(|paths| {
            paths.iter().map(|path| path.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>()
        });
        let fps = request.get("fps").and_then(Value::as_f64).unwrap_or(f64::NAN);
        let cancel = media.window(&call.client);
        let progress = progress(&call.client, "media-sequence:progress", request_id);
        let result = media.0.proxies.create_sequence(source_paths, fps, call.client.id, &cancel, progress).await;
        Ok(proxy_result(result, "video/webm", None))
    });

    router.json_with(&media, "media-animation:begin", |media, call| async move {
        let request = object(&call).ok_or_else(|| validation("media-animation:begin", "expected an object"))?;
        let fps = request.get("fps").and_then(Value::as_f64).unwrap_or(f64::NAN);
        let repeats = request
            .get("repeats")
            .and_then(Value::as_array)
            .and_then(|repeats| repeats.iter().map(Value::as_u64).collect::<Option<Vec<_>>>());
        media.window(&call.client);
        Ok(json!(media.0.proxies.begin_animation(fps, repeats, call.client.id).await?))
    });
    router.json_with(&media, "media-animation:frame", |media, call| async move {
        const CHANNEL: &str = "media-animation:frame";
        let request = object(&call).ok_or_else(|| validation(CHANNEL, "expected an object"))?;
        let token = animation_token(request, CHANNEL)?;
        let invalid = || anyhow!("Invalid animation frame chunk");
        let index = request.get("index").and_then(Value::as_u64).ok_or_else(invalid)?;
        let offset = request.get("offset").and_then(Value::as_u64).ok_or_else(invalid)?;
        let data = call.bytes(request.get("data").unwrap_or(&Value::Null)).map_err(|_| invalid())?;
        media.0.proxies.write_animation_frame(token, call.client.id, index, offset, &data).await?;
        Ok(Value::Null)
    });
    router.json_with(&media, "media-animation:finish", |media, call| async move {
        const CHANNEL: &str = "media-animation:finish";
        let request = object(&call).ok_or_else(|| validation(CHANNEL, "expected an object"))?;
        let token = animation_token(request, CHANNEL)?;
        let request_id = request_id(request, CHANNEL)?;
        let cancel = media.window(&call.client);
        let progress = progress(&call.client, "media-animation:progress", request_id);
        let result = media.0.proxies.finish_animation(token, call.client.id, &cancel, progress).await.map(|size| (token.to_owned(), size));
        Ok(proxy_result(result, "video/webm", Some("Could not import this animation")))
    });

    router.json_with(&media, "media-image:create", |media, call| async move {
        let source_path = source_request(&call, "media-image:create")?;
        let cancel = media.window(&call.client);
        let result = media.0.proxies.create_still_image(&source_path, call.client.id, &cancel).await;
        Ok(proxy_result(result, "image/png", Some("Could not convert this image")))
    });

    router.on_with(&media, "media-proxy:read", |media, call| async move {
        const CHANNEL: &str = "media-proxy:read";
        let request = object(&call).ok_or_else(|| validation(CHANNEL, "expected an object"))?;
        let token = request.get("token").and_then(Value::as_str).filter(|token| is_token(token));
        let offset = request.get("offset").and_then(Value::as_u64);
        let length = request.get("length").and_then(Value::as_u64);
        let (Some(token), Some(offset), Some(length)) = (token, offset, length) else {
            return Err(validation(CHANNEL, "invalid proxy range"));
        };
        Ok(Payload::bytes(media.0.proxies.read(token, call.client.id, offset, length).await?))
    });
    router.json_with(&media, "media-proxy:release", |media, call| async move {
        let token = call.arg(0).as_str().filter(|token| is_token(token));
        let token = token.ok_or_else(|| validation("media-proxy:release", "invalid proxy token"))?;
        media.0.proxies.release_owned(token, call.client.id).await;
        Ok(Value::Null)
    });

    router.json_with(&media, "render:start", |media, call| async move {
        let options = EncoderOptions::parse(call.arg(0))?;
        let owner = call.client.id;
        let extension = options.format.extension();
        // agent-export destinations are not ported; every export asks.
        let chosen = crate::dialog::file()
            .set_file_name(options.default_file_name())
            .add_filter("Video", &[extension])
            .save_file()
            .await;
        let Some(destination) = chosen.map(|file| file.path().to_path_buf()) else { return Ok(Value::Null) };
        if call.client.is_closed() {
            return Ok(Value::Null);
        }
        media.window(&call.client);
        let token = media.0.render.start(options, owner).await?;
        if call.client.is_closed() {
            media.0.render.release(&token, owner).await?;
            return Ok(Value::Null);
        }
        media.0.render.set_destination(&token, owner, destination)?;
        Ok(json!(token))
    });
    router.json_with(&media, "render:write", |media, call| async move {
        let request = object(&call).context("Invalid render chunk")?;
        let token = render_token(request)?;
        let data = call.bytes(request.get("data").unwrap_or(&Value::Null)).context("Invalid render chunk")?;
        let audio = request.get("audio").and_then(Value::as_bool).unwrap_or(false);
        media.0.render.write(token, call.client.id, &data, audio).await?;
        Ok(Value::Null)
    });
    router.json_with(&media, "render:finish", |media, call| async move {
        let token = render_token(object(&call).context("Unknown render job")?)?;
        let owner = call.client.id;
        let destination = media.0.render.destination(token, owner)?;
        let result = async {
            let destination = destination.context("Export destination is unavailable")?;
            let file = media.0.render.finish(token, owner).await?;
            tokio::fs::copy(&file, &destination).await.with_context(|| format!("Could not save {}", destination.display()))?;
            call.host.remember_export(owner, &destination.to_string_lossy());
            Ok(json!({ "path": destination }))
        }
        .await;
        media.0.render.release(token, owner).await?;
        result
    });
    router.json_with(&media, "render:cancel", |media, call| async move {
        let token = render_token(object(&call).context("Unknown render job")?)?;
        media.0.render.release(token, call.client.id).await?;
        Ok(Value::Null)
    });
    Ok(())
}

fn object(call: &Call) -> Option<&serde_json::Map<String, Value>> {
    call.arg(0).as_object()
}

/// `IpcValidationError`
fn validation(channel: &str, detail: &str) -> anyhow::Error {
    anyhow!("{channel}: {detail}")
}

/// `proxyRequest`: `{ sourcePath, name }` with an absolute path.
fn source_request(call: &Call, channel: &str) -> Result<String> {
    let request = object(call).ok_or_else(|| validation(channel, "expected an object"))?;
    let source_path = request.get("sourcePath").and_then(Value::as_str);
    let Some(source_path) = source_path.filter(|path| path.encode_utf16().count() <= 16_384 && Path::new(path).is_absolute()) else {
        bail!(validation(channel, "invalid source path"));
    };
    let name_length = request.get("name").and_then(Value::as_str).map(|name| name.encode_utf16().count());
    if !name_length.is_some_and(|length| (1..=1_000).contains(&length)) {
        bail!(validation(channel, "invalid file name"));
    }
    Ok(source_path.to_owned())
}

/// `REQUEST_ID = /^[a-z0-9-]{8,80}$/`, optional.
fn request_id(request: &serde_json::Map<String, Value>, channel: &str) -> Result<Option<String>> {
    match request.get("requestId") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id))
            if (8..=80).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') =>
        {
            Ok(Some(id.clone()))
        }
        Some(_) => Err(validation(channel, "invalid request id")),
    }
}

fn animation_token<'a>(request: &'a serde_json::Map<String, Value>, channel: &str) -> Result<&'a str> {
    request.get("token").and_then(Value::as_str).filter(|token| is_token(token)).ok_or_else(|| validation(channel, "invalid animation token"))
}

fn render_token(request: &serde_json::Map<String, Value>) -> Result<&str> {
    request.get("token").and_then(Value::as_str).context("Unknown render job")
}

fn progress(client: &Arc<Client>, channel: &'static str, request_id: Option<String>) -> impl FnMut(u64) + Send + use<> {
    let client = client.clone();
    move |completed| {
        if let Some(request_id) = &request_id {
            client.emit(channel, json!({ "requestId": request_id, "completed": completed }));
        }
    }
}

/// `MediaProxyResult`; conversion failures are answers, not thrown errors.
fn proxy_result(result: Result<(String, u64)>, kind: &str, context: Option<&str>) -> Value {
    match result {
        Ok((token, size)) => json!({ "ok": true, "token": token, "type": kind, "size": size }),
        Err(error) => {
            let message = error.to_string();
            let error = match context {
                Some(context) if message.is_empty() => context.to_owned(),
                Some(context) => format!("{context} · {message}"),
                None => message,
            };
            json!({ "ok": false, "error": error })
        }
    }
}
