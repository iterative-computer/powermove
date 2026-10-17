//! Imported model assets: the file loaded once per version (path, size,
//! modification time) into a [`pm_geom3d::gltf::Model`], with its images
//! decoded as Chromium decodes them for `ImageBitmapLoader`
//! (`createImageBitmap(blob, { premultiplyAlpha: 'none',
//! colorSpaceConversion: 'none' })`: straight RGBA8 as stored, no color
//! management). A texture whose image does not decode loads as no texture
//! (`loadTextureImage`'s `catch` returns `null`), so its map is absent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use pm_geom3d::gltf::{self, Model};

/// A decoded image: straight RGBA8 rows, top row first.
#[derive(Debug, PartialEq, Eq)]
pub struct DecodedImage {
    /// Content hash (the GPU texture cache key).
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An imported glTF or GLB model.
#[derive(Debug)]
pub struct GltfAsset {
    /// The file version's key.
    pub key: u64,
    pub model: Arc<Model>,
    /// Per glTF image: the decoded pixels, or `None` when Chromium would
    /// not decode it.
    pub images: Vec<Option<Arc<DecodedImage>>>,
}

impl PartialEq for GltfAsset {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl GltfAsset {
    /// Whether texture `t` loads (its image decodes).
    pub fn texture_loads(&self, t: usize) -> bool {
        self.model
            .textures
            .get(t)
            .and_then(|t| self.images.get(t.image))
            .is_some_and(Option::is_some)
    }
}

fn hash(parts: &[&[u8]]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().unwrap())
}

type Version = (PathBuf, u64, Option<std::time::SystemTime>);

type Loaded = Mutex<HashMap<Version, Result<Arc<GltfAsset>, String>>>;

fn cache() -> &'static Loaded {
    static C: OnceLock<Loaded> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The model at `path`, loaded once per file version. `Err` when the file
/// does not load (today's importer refuses it, so the asset has no scene
/// and draws as the missing-model box).
pub fn load(path: &Path) -> Result<Arc<GltfAsset>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let version = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(r) = cache().lock().unwrap().get(&version) {
        return r.clone();
    }
    let result = std::fs::read(path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|bytes| from_bytes(&bytes));
    let mut c = cache().lock().unwrap();
    c.retain(|k, _| k.0 != version.0);
    if c.len() >= 64 {
        c.clear();
    }
    c.insert(version, result.clone());
    result
}

/// A model from file bytes (keyed by their content).
pub fn from_bytes(bytes: &[u8]) -> Result<Arc<GltfAsset>, String> {
    let model = gltf::load(bytes).map_err(|e| e.0)?;
    let images = model
        .images
        .iter()
        .map(|i| decode(&i.bytes).ok().map(Arc::new))
        .collect();
    Ok(Arc::new(GltfAsset {
        key: hash(&[b"gltf asset 1", bytes]),
        model: Arc::new(model),
        images,
    }))
}

/// The image formats Chromium's `createImageBitmap` decodes.
fn chromium_decodes(format: pm_media::image::ImageFormat) -> bool {
    use pm_media::image::ImageFormat as F;
    matches!(
        format,
        F::Png | F::Jpeg | F::WebP | F::Avif | F::Gif | F::Bmp | F::Ico
    )
}

/// Decodes embedded image bytes through `pm-media` (the engine's still
/// readers: the `image` crate for PNG, JPEG, WebP, GIF, BMP, ICO; ImageIO
/// for AVIF), sniffing the content as the browser does.
pub fn decode(bytes: &[u8]) -> Result<DecodedImage, String> {
    let format = pm_media::image::sniff(&bytes[..bytes.len().min(64)]);
    if !chromium_decodes(format) {
        return Err(format!("{format:?} images do not decode in Chromium"));
    }
    let key = hash(&[b"gltf image 1", bytes]);
    // `pm-media` reads files: the bytes go to a content-addressed file in
    // the temporary directory (written once).
    let dir = std::env::temp_dir().join("powermove-gltf-images");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{key:016x}.img"));
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(bytes.len() as u64) {
        let tmp = dir.join(format!("{key:016x}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    let pool = pm_media::pool::FramePool::new(0);
    let still = pm_media::image::read(&path, &pool).map_err(|e| e.to_string())?;
    let f = &still.frame.format;
    if f.planes.len() != 1 {
        return Err(format!("glTF image layout {}", f.label));
    }
    let (w, h) = (f.width, f.height);
    let plane = f.planes[0].format;
    let premultiplied = f.rgba_premultiplied;
    let mut out = vec![0u8; w * h * 4];
    still
        .frame
        .with_planes(|planes| {
            let p = &planes[0];
            for y in 0..h {
                match plane {
                    pm_media::frame::PlaneFormat::Rgba8 => {
                        // SAFETY: the plane holds `height` rows of `stride` bytes.
                        let row =
                            unsafe { std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * 4) };
                        out[y * w * 4..(y + 1) * w * 4].copy_from_slice(row);
                    }
                    pm_media::frame::PlaneFormat::Rgba16 => {
                        // SAFETY: as above, 8 bytes per pixel.
                        let row = unsafe {
                            std::slice::from_raw_parts(p.ptr.add(y * p.stride) as *const u16, w * 4)
                        };
                        for (o, v) in out[y * w * 4..(y + 1) * w * 4].iter_mut().zip(row) {
                            // 16 to 8 bits with rounding (Skia's conversion).
                            *o = ((u32::from(*v) * 255 + 32767) / 65535) as u8;
                        }
                    }
                    _ => {}
                }
            }
        })
        .map_err(|e| e.to_string())?;
    if !matches!(
        plane,
        pm_media::frame::PlaneFormat::Rgba8 | pm_media::frame::PlaneFormat::Rgba16
    ) {
        return Err(format!("glTF image plane {plane:?}"));
    }
    if premultiplied {
        // Straight alpha (`premultiplyAlpha: 'none'`).
        for px in out.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            if a > 0 && a < 255 {
                for c in &mut px[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
    Ok(DecodedImage {
        key,
        width: w as u32,
        height: h as u32,
        rgba: out,
    })
}
