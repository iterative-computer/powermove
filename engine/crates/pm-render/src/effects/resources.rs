//! Bounded worker asset leases. A prepared chain owns its exact decoded bytes;
//! replacement or deletion cannot retarget an in-flight effect to another file.
//! A file is read when its stamp changes and a table uploads once, so a frame
//! costs one look at each file and no copy.
use pm_eval::node;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
};
#[derive(PartialEq)]
pub struct Asset {
    pub key: [u8; 32],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    pub lut: Option<pm_color::lut::GpuLut>,
    pub vector: Option<crate::vector::VectorSource>,
    pub brush: bool,
}
impl std::fmt::Debug for Asset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EffectAsset")
            .field(&self.key)
            .field(&self.width)
            .field(&self.height)
            .finish()
    }
}
/// An asset id and whether it's read as a LUT.
type Request = (String, bool);

#[derive(Default)]
pub(crate) struct Assets {
    pub prepared: BTreeMap<Request, Result<Arc<Asset>, String>>,
    /// The file behind each prepared entry, as it was on disk when read.
    sources: BTreeMap<Request, (PathBuf, (u64, u128))>,
    /// Prepared assets' textures, by content. Only what `prepared` holds stays.
    uploads: HashMap<[u8; 32], pm_gpu::Target>,
    /// The effect asset references of the last document walked, by its
    /// edition: a frame of an unchanged document skips the walk.
    walked: Option<(u64, BTreeSet<Request>)>,
    /// A new edition re-walks only the subtrees its edits copied.
    memo: crate::doc_walk::SubtreeMemo<Request>,
}
impl Assets {
    /// `asset`'s texture, uploaded on first use.
    pub fn texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset: &Asset,
    ) -> pm_gpu::Target {
        // A brush's mask changes per frame, and its key is only that mask's.
        if !asset.brush
            && let Some(target) = self.uploads.get(&asset.key)
        {
            return target.clone();
        }
        let target = pm_gpu::Target::new(
            device,
            "pm effect asset lease",
            asset.width,
            asset.height,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let data = asset
            .lut
            .as_ref()
            .map_or(asset.rgba.as_slice(), |c| c.rgba.as_slice());
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_texture(
            target.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(asset.width * 16),
                rows_per_image: Some(asset.height),
            },
            target.texture.size(),
        );
        if !asset.brush {
            self.uploads.insert(asset.key, target.clone());
        }
        target
    }

    pub fn prepare(
        &mut self,
        document: &pm_model::Document,
        resolver: &dyn crate::assets::AssetResolver,
    ) {
        /// One node's asset references: an effect's `inputs` of kind asset.
        fn visit(n: &pm_model::Node, out: &mut Vec<Request>) {
            if let pm_model::NodeKind::Object(_) = n.kind() {
                let lut = node::is_str(node::member(Some(n), "type"), "applyColorLut");
                if let Some(inputs) = node::member(Some(n), "inputs")
                    && let pm_model::NodeKind::Object(inputs) = inputs.kind()
                {
                    for input in inputs.values() {
                        if node::is_str(node::member(Some(input), "kind"), "asset")
                            && let Some(id) = node::string(node::member(Some(input), "assetId"))
                        {
                            out.push((id.to_string_lossy(), lut));
                        }
                    }
                }
            }
        }
        let Some(root) = document.root() else {
            self.prepared.clear();
            self.sources.clear();
            self.uploads.clear();
            return;
        };
        let requests = match self.walked.take() {
            Some((edition, requests)) if edition == document.edition() => requests,
            _ => self.memo.walk(root, &mut visit).into_iter().collect(),
        };
        self.walked = Some((document.edition(), requests.clone()));
        let old = std::mem::take(&mut self.prepared);
        let sources = std::mem::take(&mut self.sources);
        for (id, lut) in requests {
            let loaded = (|| {
                let record = node::member(node::member(Some(root), "assets"), &id);
                let kind = node::string(node::member(record, "kind")).map(|s| s.to_string_lossy());
                // The sentences a person reads when the file can't be used.
                let what = if lut { "LUT" } else { "image" };
                let name = node::string(node::member(record, "name"))
                    .map(|s| s.to_string_lossy())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| id.clone());
                let unreadable =
                    |why: String| format!("The {what} “{name}” couldn’t be read. {why}");
                if kind.as_deref() != Some(if lut { "lut" } else { "image" }) {
                    return Err(format!(
                        "“{name}” isn’t {} file.",
                        if lut { "a LUT" } else { "an image" }
                    ));
                }
                let path = resolver.path(&id, record).ok_or_else(|| {
                    format!("The {what} “{name}” can’t be found. Relink it or remove the effect.")
                })?;
                let request = (id.clone(), lut);
                let stamp = crate::media::stamp(&path);
                if let Some(stamp) = stamp {
                    let source = (path.clone(), stamp);
                    let unchanged = sources.get(&request) == Some(&source);
                    self.sources.insert(request.clone(), source);
                    pm_perf::cache(
                        pm_perf::Counter::EffectAssetHits,
                        pm_perf::Counter::EffectAssetMisses,
                        unchanged,
                    );
                    if unchanged && let Some(prepared) = old.get(&request) {
                        return prepared.clone();
                    }
                }
                use std::io::Read;
                let limit = if lut {
                    pm_color::lut::MAX_FILE_BYTES
                } else {
                    16 * 1024 * 1024
                };
                let mut bytes = vec![];
                std::fs::File::open(path)
                    .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
                    .map_err(|_| {
                        format!(
                            "The {what} “{name}” can’t be found. Relink it or remove the effect."
                        )
                    })?;
                if bytes.len() as u64 > limit {
                    return Err(format!(
                        "The {what} “{name}” is larger than {} MiB.",
                        limit / 1024 / 1024
                    ));
                }
                let key = Sha256::digest(&bytes).into();
                if let Some(Ok(asset)) = old.get(&request)
                    && asset.key == key
                {
                    return Ok(asset.clone());
                }
                let asset = if lut {
                    let cube = std::str::from_utf8(&bytes)
                        .map_err(|_| unreadable("It isn’t a text file.".into()))
                        .and_then(|text| {
                            let format = node::string(node::member(record, "format"))
                                .map(|s| s.to_string_lossy())
                                .unwrap_or_default();
                            pm_color::lut::CubeLut::parse_format(&format, text)
                                .and_then(|lut| lut.gpu())
                                .map_err(|e| unreadable(e.to_string()))
                        })?;
                    Asset {
                        key,
                        width: cube.width,
                        height: cube.height,
                        rgba: vec![],
                        lut: Some(cube),
                        vector: None,
                        brush: false,
                    }
                } else {
                    let image = crate::scene3d::gltf::asset::decode(&bytes).map_err(unreadable)?;
                    if image.width > 16384
                        || image.height > 16384
                        || image.rgba.len() > 256 * 1024 * 1024
                    {
                        return Err(format!("The image “{name}” is too large for an effect."));
                    }
                    // FBO rows for shared sample_<key>; image alpha is straight.
                    let mut rgba = Vec::with_capacity(image.rgba.len());
                    for row in image.rgba.chunks_exact(image.width as usize * 4).rev() {
                        for p in row.chunks_exact(4) {
                            let a = p[3] as f32 / 255.;
                            rgba.extend([
                                p[0] as f32 / 255. * a,
                                p[1] as f32 / 255. * a,
                                p[2] as f32 / 255. * a,
                                a,
                            ]);
                        }
                    }
                    Asset {
                        key,
                        width: image.width,
                        height: image.height,
                        rgba,
                        lut: None,
                        vector: None,
                        brush: false,
                    }
                };
                Ok(Arc::new(asset))
            })();
            self.prepared.insert((id, lut), loaded);
        }
        let held: BTreeSet<_> = self.prepared.values().flatten().map(|a| a.key).collect();
        self.uploads.retain(|key, _| held.contains(key));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub effect: String,
    pub name: String,
    pub request: pm_fx_defs::inputs::Request,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Producer {
    /// Source framebuffer cost of this producer, including nested leases.
    pub source_bytes: u64,
    pub source_extent: Option<pm_fx_defs::spatial::Extent>,
    pub request: pm_fx_defs::inputs::Request,
    pub plan: Arc<crate::plan::CompPlan>,
    pub owner: super::plan::Owner,
}
#[derive(Clone)]
pub(crate) struct Ready {
    pub source_extent: Option<pm_fx_defs::spatial::Extent>,
    pub request: pm_fx_defs::inputs::Request,
    pub target: pm_gpu::Target,
    pub owner: super::plan::Owner,
    pub origin: [f64; 2],
}
pub(crate) fn from_uv(owner: &super::plan::Owner) -> [f32; 9] {
    let m = owner.uv.map(f64::from);
    // A normalized UV matrix can have a very small valid determinant for
    // large sources. The document transform's absolute threshold is not a
    // valid singularity test in these units.
    let det = m[0] * m[4] - m[1] * m[3];
    [
        (m[4] / det) as f32,
        (-m[1] / det) as f32,
        0.,
        (-m[3] / det) as f32,
        (m[0] / det) as f32,
        0.,
        ((m[3] * m[7] - m[4] * m[6]) / det) as f32,
        ((m[1] * m[6] - m[0] * m[7]) / det) as f32,
        1.,
    ]
}

impl Asset {
    pub(crate) fn request(&self) -> pm_fx_defs::inputs::Request {
        pm_fx_defs::inputs::Request::new(
            format!("@brush:{:?}", self.key),
            pm_fx_defs::inputs::Stage::Source,
            pm_fx_defs::inputs::Space::Composition,
            None,
            None,
        )
        .unwrap()
    }
}
