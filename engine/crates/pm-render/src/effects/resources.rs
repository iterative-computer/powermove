//! Bounded worker asset leases. A prepared chain owns its exact decoded bytes;
//! replacement or deletion cannot retarget an in-flight effect to another file.
use pm_eval::node;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(PartialEq)]
pub struct Asset {
    pub key: [u8; 32],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
    pub lut: Option<pm_fx_defs::lut::Cube>,
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
#[derive(Default)]
pub(crate) struct Assets(pub BTreeMap<(String, bool), Result<Arc<Asset>, String>>);
impl Assets {
    pub fn prepare(
        &mut self,
        document: &pm_model::Document,
        resolver: &dyn crate::assets::AssetResolver,
    ) {
        let mut requests = BTreeSet::new();
        fn walk(n: &pm_model::Node, out: &mut BTreeSet<(String, bool)>) {
            match n.kind() {
                pm_model::NodeKind::Object(map) => {
                    let lut = node::is_str(node::member(Some(n), "type"), "applyColorLut");
                    if let Some(inputs) = node::member(Some(n), "inputs")
                        && let pm_model::NodeKind::Object(inputs) = inputs.kind()
                    {
                        for input in inputs.values() {
                            if node::is_str(node::member(Some(input), "kind"), "asset")
                                && let Some(id) = node::string(node::member(Some(input), "assetId"))
                            {
                                out.insert((id.to_string_lossy(), lut));
                            }
                        }
                    }
                    for child in map.values() {
                        walk(child, out);
                    }
                }
                pm_model::NodeKind::Array(a) => {
                    for child in a.iter() {
                        walk(child, out);
                    }
                }
                _ => {}
            }
        }
        let Some(root) = document.root() else {
            self.0.clear();
            return;
        };
        walk(root, &mut requests);
        let old = std::mem::take(&mut self.0);
        for (id, lut) in requests {
            let loaded = (|| {
                let record = node::member(node::member(Some(root), "assets"), &id);
                let kind = node::string(node::member(record, "kind")).map(|s| s.to_string_lossy());
                if kind.as_deref() != Some(if lut { "lut" } else { "image" }) {
                    return Err(format!(
                        "Effect asset requires {} metadata: {id}",
                        if lut { "lut" } else { "image" }
                    ));
                }
                let path = resolver
                    .path(&id, record)
                    .ok_or_else(|| format!("Effect asset is offline: {id}"))?;
                use std::io::Read;
                let mut bytes = vec![];
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err("Effect asset exceeds 16 MiB".into());
                }
                let key = Sha256::digest(&bytes).into();
                if let Some(Ok(asset)) = old.get(&(id.clone(), lut))
                    && asset.key == key
                {
                    return Ok(asset.clone());
                }
                let asset = if lut {
                    let cube = pm_fx_defs::lut::parse(
                        std::str::from_utf8(&bytes).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
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
                    let image = crate::scene3d::gltf::asset::decode(&bytes)?;
                    if image.width > 16384
                        || image.height > 16384
                        || image.rgba.len() > 256 * 1024 * 1024
                    {
                        return Err("Effect image exceeds texture budget".into());
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
            self.0.insert((id, lut), loaded);
        }
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
