//! Playback proxies (`MediaProxyService` in media-proxy.ts): converted files
//! the renderer reads back in chunks by token until it releases them.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use tokio_util::sync::CancellationToken;

use super::convert::Converters;
use super::sequence::{
    MAX_SAFE_INTEGER, MAX_SEQUENCE_FRAMES, is_video_extension, media_extension, needs_image_conversion, ordered_sequence,
    valid_sequence_fps,
};

pub const MAX_PROXY_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// A decoded animation frame stays well under this even at 4K.
const MAX_ANIMATION_FRAME_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STILL_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
const WEBM_SIGNATURE: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];

/// `/^[a-f0-9]{32}$/`
pub fn is_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

struct Upload {
    total: u64,
    received: u64,
    finishing: bool,
}

struct Animation {
    fps: f64,
    repeats: Vec<u64>,
    written: Vec<u64>,
    finishing: bool,
}

struct State {
    file: PathBuf,
    size: u64,
    upload: Option<Upload>,
    animation: Option<Animation>,
}

struct Entry {
    owner: u64,
    directory: PathBuf,
    state: Mutex<State>,
    /// Chunk writes hold a read guard; finishing and release take the write
    /// lock, so they start only after every accepted chunk is on disk.
    io: Arc<RwLock<()>>,
}

pub struct ProxyService {
    temp_root: PathBuf,
    convert: Converters,
    entries: Mutex<HashMap<String, Arc<Entry>>>,
}

impl ProxyService {
    pub fn new(temp_root: PathBuf, convert: Converters) -> Self {
        Self { temp_root, convert, entries: Mutex::default() }
    }

    fn entry(&self, token: &str, owner: u64) -> Option<Arc<Entry>> {
        self.entries.lock().get(token).filter(|entry| entry.owner == owner).cloned()
    }

    fn insert(&self, owner: u64, directory: PathBuf, state: State) -> String {
        let token = new_token();
        let entry = Arc::new(Entry { owner, directory, state: Mutex::new(state), io: Arc::default() });
        self.entries.lock().insert(token.clone(), entry);
        token
    }

    fn insert_finished(&self, owner: u64, directory: PathBuf, file: PathBuf, size: u64) -> String {
        self.insert(owner, directory, State { file, size, upload: None, animation: None })
    }

    pub async fn begin_preview(&self, total: u64, owner: u64) -> Result<String> {
        if !(1..=MAX_SAFE_INTEGER).contains(&total) {
            bail!("Invalid preview source size");
        }
        let directory = make_temp_dir(&self.temp_root, "powermove-preview-").await?;
        let state = State {
            file: directory.join("source"),
            size: 0,
            upload: Some(Upload { total, received: 0, finishing: false }),
            animation: None,
        };
        Ok(self.insert(owner, directory, state))
    }

    pub async fn write_preview(&self, token: &str, owner: u64, offset: u64, data: &[u8]) -> Result<()> {
        let invalid = || anyhow::anyhow!("Invalid preview source chunk");
        let entry = self.entry(token, owner).ok_or_else(invalid)?;
        let (file, _guard) = {
            let mut state = entry.state.lock();
            let length = data.len() as u64;
            let upload = state.upload.as_mut().ok_or_else(invalid)?;
            if upload.finishing
                || offset != upload.received
                || !(1..=MAX_PROXY_CHUNK_BYTES).contains(&length)
                || offset + length > upload.total
            {
                return Err(invalid());
            }
            upload.received += length;
            (state.file.clone(), entry.io.clone().try_read_owned().map_err(|_| invalid())?)
        };
        write_at(&file, offset, data).await
    }

    pub async fn finish_preview(&self, token: &str, owner: u64, cancel: &CancellationToken) -> Result<u64> {
        let entry = self.entry(token, owner).context("Incomplete preview source")?;
        {
            let mut state = entry.state.lock();
            match state.upload.as_mut() {
                Some(upload) if !upload.finishing && upload.received == upload.total => upload.finishing = true,
                _ => bail!("Incomplete preview source"),
            }
        }
        let result = self.convert_preview(&entry, cancel).await;
        if result.is_err() {
            self.release(token).await;
        }
        result
    }

    async fn convert_preview(&self, entry: &Entry, cancel: &CancellationToken) -> Result<u64> {
        let _writes_done = entry.io.write().await;
        let mut source = entry.state.lock().file.clone();
        // Detect the WebM container signature without trusting a user filename.
        let mut prefix = [0u8; 4];
        let mut file = tokio::fs::File::open(&source).await?;
        let read = file.read(&mut prefix).await?;
        drop(file);
        if read == 4 && prefix == WEBM_SIGNATURE {
            let named = source.with_extension("webm");
            tokio::fs::rename(&source, &named).await?;
            source = named;
        }
        let output = entry.directory.join("preview.webm");
        self.convert.preview(&source, &output, cancel).await?;
        let size = nonempty_file(&output, "The preview was empty").await?;
        let mut state = entry.state.lock();
        state.file = output;
        state.size = size;
        state.upload = None;
        Ok(size)
    }

    pub async fn create(&self, source_path: &str, owner: u64, cancel: &CancellationToken) -> Result<(String, u64)> {
        let source_path = Path::new(source_path);
        let extension = source_path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !source_path.is_absolute() || !is_video_extension(&extension) {
            bail!("This video container is not one Powermove can convert");
        }
        let resolved = readable_file(source_path, "The selected video file is not readable").await?.0;
        let directory = make_temp_dir(&self.temp_root, "powermove-media-proxy-").await?;
        let output = directory.join("playback.webm");
        let converted = async {
            self.convert.playback(&resolved, &output, cancel).await?;
            nonempty_file(&output, "The playback proxy was empty").await
        }
        .await;
        self.finish_new(owner, directory, output, converted).await
    }

    pub async fn create_sequence(
        &self,
        source_paths: Option<Vec<String>>,
        fps: f64,
        owner: u64,
        cancel: &CancellationToken,
        on_progress: impl FnMut(u64) + Send,
    ) -> Result<(String, u64)> {
        if !valid_sequence_fps(fps) {
            bail!("Frame rate must be between 1 and 240 fps");
        }
        let source_paths = source_paths
            .filter(|paths| {
                paths.len() >= 2 && paths.iter().all(|source| source.encode_utf16().count() <= 16_384 && Path::new(source).is_absolute())
            })
            .context("Choose local numbered image files")?;
        let frames = ordered_sequence(source_paths, |source| file_name(source))?;
        let directory = make_temp_dir(&self.temp_root, "powermove-image-sequence-").await?;
        let output = directory.join("sequence.webm");
        let converted = async {
            let extension = format!(".{}", media_extension(file_name(&frames[0])));
            // Only generated names enter FFmpeg's pattern; user filenames never
            // become demuxer directives. Links avoid copying the sequence where
            // the volume allows them.
            for (index, frame) in frames.iter().enumerate() {
                let message = format!("Could not read {}", file_name(frame));
                let (source, _) = readable_file(Path::new(frame), &message).await?;
                link_or_copy(&source, &directory.join(format!("frame-{index:08}{extension}"))).await?;
            }
            let pattern = directory.join(format!("frame-%08d{extension}"));
            self.convert.sequence(&pattern, fps, frames.len() as u64, &output, cancel, on_progress).await?;
            nonempty_file(&output, "The image sequence was empty").await
        }
        .await;
        self.finish_new(owner, directory, output, converted).await
    }

    /// Animated GIF, APNG and animated WebP have no FFmpeg demuxer worth
    /// trusting, but Chromium decodes all of them. The renderer sends the
    /// frames it decoded, each with the number of sequence slots it has to
    /// fill, and they encode through the image sequence converter.
    pub async fn begin_animation(&self, fps: f64, repeats: Option<Vec<u64>>, owner: u64) -> Result<String> {
        if !valid_sequence_fps(fps) {
            bail!("Frame rate must be between 1 and 240 fps");
        }
        let repeats = repeats
            .filter(|repeats| !repeats.is_empty() && repeats.iter().all(|count| (1..=MAX_SEQUENCE_FRAMES).contains(count)))
            .context("Invalid animation frame timing")?;
        if repeats.iter().sum::<u64>() > MAX_SEQUENCE_FRAMES {
            bail!("This animation has too many frames to import");
        }
        let directory = make_temp_dir(&self.temp_root, "powermove-animation-").await?;
        let written = vec![0; repeats.len()];
        let state = State {
            file: PathBuf::new(),
            size: 0,
            upload: None,
            animation: Some(Animation { fps, repeats, written, finishing: false }),
        };
        Ok(self.insert(owner, directory, state))
    }

    pub async fn write_animation_frame(&self, token: &str, owner: u64, index: u64, offset: u64, data: &[u8]) -> Result<()> {
        let invalid = || anyhow::anyhow!("Invalid animation frame chunk");
        let entry = self.entry(token, owner).ok_or_else(invalid)?;
        let _guard: OwnedRwLockReadGuard<()> = {
            let mut state = entry.state.lock();
            let length = data.len() as u64;
            let animation = state.animation.as_mut().ok_or_else(invalid)?;
            let slot = usize::try_from(index).ok().filter(|slot| *slot < animation.repeats.len()).ok_or_else(invalid)?;
            if animation.finishing
                || !(1..=MAX_PROXY_CHUNK_BYTES).contains(&length)
                || offset != animation.written[slot]
                || offset + length > MAX_ANIMATION_FRAME_BYTES
            {
                return Err(invalid());
            }
            animation.written[slot] = offset + length;
            entry.io.clone().try_read_owned().map_err(|_| invalid())?
        };
        // Only generated names reach the filesystem; the decoded frame carries none.
        write_at(&entry.directory.join(format!("source-{index:08}.png")), offset, data).await
    }

    pub async fn finish_animation(
        &self,
        token: &str,
        owner: u64,
        cancel: &CancellationToken,
        on_progress: impl FnMut(u64) + Send,
    ) -> Result<u64> {
        let entry = self.entry(token, owner).context("Unknown animation import")?;
        let (fps, repeats) = {
            let mut state = entry.state.lock();
            let animation = state.animation.as_mut().filter(|animation| !animation.finishing).context("Unknown animation import")?;
            if animation.written.iter().any(|bytes| *bytes < 1) {
                bail!("The animation is missing frames");
            }
            animation.finishing = true;
            (animation.fps, animation.repeats.clone())
        };
        let result = async {
            let _writes_done = entry.io.write().await;
            // Repeating a frame holds it on screen for its own delay at a fixed rate.
            let mut slot = 0u64;
            for (index, count) in repeats.iter().enumerate() {
                let source = entry.directory.join(format!("source-{index:08}.png"));
                for _ in 0..*count {
                    link_or_copy(&source, &entry.directory.join(format!("frame-{slot:08}.png"))).await?;
                    slot += 1;
                }
            }
            let output = entry.directory.join("animation.webm");
            self.convert.sequence(&entry.directory.join("frame-%08d.png"), fps, slot, &output, cancel, on_progress).await?;
            let size = nonempty_file(&output, "The converted animation was empty").await?;
            let mut state = entry.state.lock();
            state.file = output;
            state.size = size;
            state.animation = None;
            Ok(size)
        }
        .await;
        if result.is_err() {
            self.release(token).await;
        }
        result
    }

    /// TIFF and HEIF stills become a PNG the renderer can decode like any other.
    pub async fn create_still_image(&self, source_path: &str, owner: u64, cancel: &CancellationToken) -> Result<(String, u64)> {
        let extension = media_extension(source_path);
        let source_path = Path::new(source_path);
        if !source_path.is_absolute() || !needs_image_conversion(&extension) {
            bail!("This image format does not need converting");
        }
        let (resolved, size) = readable_file(source_path, "The selected image file is not readable").await?;
        if size > MAX_STILL_IMAGE_BYTES {
            bail!("This image file is too large to convert");
        }
        let directory = make_temp_dir(&self.temp_root, "powermove-image-").await?;
        let output = directory.join("image.png");
        let converted = async {
            // The requested name picks the decoder, never wherever a link points.
            self.convert.still(&resolved, &extension, &output, cancel).await?;
            nonempty_file(&output, "The converted image was empty").await
        }
        .await;
        self.finish_new(owner, directory, output, converted).await
    }

    async fn finish_new(&self, owner: u64, directory: PathBuf, output: PathBuf, converted: Result<u64>) -> Result<(String, u64)> {
        match converted {
            Ok(size) => Ok((self.insert_finished(owner, directory, output, size), size)),
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&directory).await;
                Err(error)
            }
        }
    }

    pub async fn read(&self, token: &str, owner: u64, offset: u64, length: u64) -> Result<Bytes> {
        if !is_token(token) {
            bail!("Unknown playback proxy");
        }
        let entry = self.entry(token, owner).context("The playback proxy has expired")?;
        let (file, size) = {
            let state = entry.state.lock();
            (state.file.clone(), state.size)
        };
        if offset > size || !(1..=MAX_PROXY_CHUNK_BYTES).contains(&length) {
            bail!("Invalid playback proxy range");
        }
        let wanted = length.min(size - offset);
        if wanted == 0 {
            return Ok(Bytes::new());
        }
        let mut handle = tokio::fs::File::open(&file).await?;
        handle.seek(SeekFrom::Start(offset)).await?;
        let mut buffer = Vec::with_capacity(wanted as usize);
        handle.take(wanted).read_to_end(&mut buffer).await?;
        Ok(Bytes::from(buffer))
    }

    pub async fn release(&self, token: &str) {
        if !is_token(token) {
            return;
        }
        let Some(entry) = self.entries.lock().remove(token) else { return };
        let _writes_done = entry.io.write().await;
        let _ = tokio::fs::remove_dir_all(&entry.directory).await;
    }

    /// `release` scoped to the caller's own proxies.
    pub async fn release_owned(&self, token: &str, owner: u64) {
        if self.entry(token, owner).is_some() {
            self.release(token).await;
        }
    }

    pub async fn release_owner(&self, owner: u64) {
        let tokens: Vec<String> =
            self.entries.lock().iter().filter(|(_, entry)| entry.owner == owner).map(|(token, _)| token.clone()).collect();
        for token in tokens {
            self.release(&token).await;
        }
    }
}

fn file_name(path: &str) -> &str {
    Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path)
}

/// `mkdtemp(path.join(root, prefix))`
pub async fn make_temp_dir(root: &Path, prefix: &str) -> Result<PathBuf> {
    tokio::fs::create_dir_all(root).await?;
    let directory = root.join(format!("{prefix}{}", &new_token()[..12]));
    tokio::fs::create_dir(&directory).await?;
    Ok(directory)
}

/// Resolve links and require a non-empty regular file; returns its size.
async fn readable_file(path: &Path, message: &str) -> Result<(PathBuf, u64)> {
    let resolved = tokio::fs::canonicalize(path).await.with_context(|| message.to_owned())?;
    let resolved = without_verbatim_prefix(resolved);
    let info = tokio::fs::metadata(&resolved).await.with_context(|| message.to_owned())?;
    if !info.is_file() || info.len() == 0 {
        bail!("{message}");
    }
    Ok((resolved, info.len()))
}

async fn nonempty_file(path: &Path, message: &str) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(info) if info.is_file() && info.len() > 0 => Ok(info.len()),
        _ => bail!("{message}"),
    }
}

/// `canonicalize` returns `\\?\C:\…` on Windows; hand ffmpeg the plain form.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Windows needs admin rights for symlinks; a hard link is free on the same
/// volume, and a copy works everywhere else (or past NTFS's 1024-link limit).
async fn link_or_copy(source: &Path, destination: &Path) -> Result<()> {
    if tokio::fs::hard_link(source, destination).await.is_err() {
        tokio::fs::copy(source, destination).await.with_context(|| format!("Could not stage {}", destination.display()))?;
    }
    Ok(())
}

/// Chunks may arrive on concurrent tasks, so each writes at its own offset.
async fn write_at(path: &Path, offset: u64, data: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(path).await?;
    file.seek(SeekFrom::Start(offset)).await?;
    file.write_all(data).await?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_32_lowercase_hex() {
        assert!(is_token(&new_token()));
        assert!(!is_token("ABCDEF0123456789abcdef0123456789"));
        assert!(!is_token("../../etc"));
    }

    #[test]
    fn strips_only_drive_verbatim_prefixes() {
        assert_eq!(without_verbatim_prefix(PathBuf::from(r"\\?\C:\a\b.png")), PathBuf::from(r"C:\a\b.png"));
        assert_eq!(without_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share")), PathBuf::from(r"\\?\UNC\server\share"));
    }

    #[tokio::test]
    async fn preview_chunks_must_arrive_in_order_and_fill_the_upload() {
        let temp = tempfile::tempdir().unwrap();
        let service = ProxyService::new(temp.path().into(), Converters { ffmpeg: "missing-ffmpeg".into() });
        let token = service.begin_preview(6, 1).await.unwrap();
        assert!(service.write_preview(&token, 1, 3, b"def").await.is_err());
        assert!(service.write_preview(&token, 2, 0, b"abc").await.is_err(), "another window cannot write");
        service.write_preview(&token, 1, 0, b"abc").await.unwrap();
        assert!(service.write_preview(&token, 1, 3, b"defg").await.is_err(), "past the declared size");
        service.write_preview(&token, 1, 3, b"def").await.unwrap();
        let directory = service.entry(&token, 1).unwrap().directory.clone();
        assert_eq!(std::fs::read(directory.join("source")).unwrap(), b"abcdef");
        service.release_owner(1).await;
        assert!(!directory.exists());
    }
}
