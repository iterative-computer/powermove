//! Viewer-only GPU memory cache. Export and snapshot renderers never call it.
//! Every entry is a copy, so later renders cannot mutate cached pixels.
use std::path::PathBuf;

use pm_cache::MemoryCache;
use pm_gpu::Target;
use pm_protocol::common::Ticks;
use pm_render::{LayerQuad, RenderRequest};

use super::document::MediaMap;

#[derive(Clone)]
pub(super) struct CachedFrame {
    pub color: Target,
    pub quads: Vec<LayerQuad>,
    pub representation: pm_protocol::frame::Representation,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Key {
    inputs: String,
    files: Vec<(String, Option<PathBuf>, Option<(u64, u128)>)>,
}

pub(super) struct PreviewCache {
    namespace: Option<(usize, u64, u64)>,
    /// Whether this document revision draws identically while playing and
    /// paused (no motion-blurred and no level 2 resampled layer anywhere).
    playback_invariant: Option<bool>,
    frames: MemoryCache<Key, CachedFrame>,
}

/// Playback differs from a paused frame only through motion blur samples
/// (layers whose `mblur` is set) and the level 2 resampler (layers at
/// compatibility level 2). Without either, both renders are the same pixels,
/// so a remembered playback frame serves the paused frame at that time, as
/// today's compositor reuses it.
fn playback_invariant(root: &pm_model::Node) -> bool {
    fn layer_varies(layer: &pm_model::Node) -> bool {
        let mblur = layer.get("mblur").is_some_and(|m| match m.kind() {
            pm_model::NodeKind::Null => false,
            pm_model::NodeKind::Bool(b) => *b,
            pm_model::NodeKind::Number(n) => *n != 0.0,
            // Animated or expression-driven switches may turn on.
            _ => true,
        });
        mblur || pm_render::sampling::is_level2(layer)
    }
    fn layers_vary(owner: &pm_model::Node) -> bool {
        match owner.get("layers").map(pm_model::Node::kind) {
            Some(pm_model::NodeKind::Array(layers)) => layers.iter().any(layer_varies),
            _ => false,
        }
    }
    if layers_vary(root) {
        return false;
    }
    match root.get("comps").map(pm_model::Node::kind) {
        Some(pm_model::NodeKind::Object(comps)) => !comps.values().any(layers_vary),
        _ => true,
    }
}

impl PreviewCache {
    pub fn new() -> Self {
        Self {
            namespace: None,
            playback_invariant: None,
            frames: MemoryCache::new(0, 16_384),
        }
    }

    pub fn budget(&mut self, bytes: usize) {
        self.frames.set_budget(bytes);
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.namespace = None;
        self.playback_invariant = None;
    }
    pub fn stats(&self) -> pm_cache::Stats {
        self.frames.stats()
    }

    pub fn key(
        &mut self,
        doc: &pm_model::Document,
        owner: u64,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        media: &MediaMap,
        representation: &str,
    ) -> Key {
        let namespace = (
            doc.root().map_or(0, |r| r as *const _ as usize),
            doc.revision(),
            owner,
        );
        if self.namespace != Some(namespace) {
            self.clear();
            self.namespace = Some(namespace);
        }
        let invariant = *self
            .playback_invariant
            .get_or_insert_with(|| doc.root().is_none_or(playback_invariant));
        let normalized;
        let request = if invariant {
            let mut paused = request.clone();
            paused.sampling = pm_render::sampling::Sampling::Final;
            paused.motion_blur = pm_render::motion::MotionBlur::Kind(
                pm_protocol::frame::MotionBlurKind::PreviewPaused,
            );
            normalized = paused;
            &normalized
        } else {
            request
        };
        Key {
            // The complete render request covers scale/ROI, precision, resampler,
            // shutter samples, view override, background and Blender quality.
            inputs: format!("{composition:?}:{time:?}:{request:?}:{representation}"),
            files: media
                .0
                .iter()
                .map(|(asset, path)| {
                    (
                        asset.clone(),
                        path.clone(),
                        path.as_deref().and_then(|path| {
                            let meta = std::fs::metadata(path).ok()?;
                            Some((
                                meta.len(),
                                meta.modified()
                                    .ok()?
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .ok()?
                                    .as_nanos(),
                            ))
                        }),
                    )
                })
                .collect(),
        }
    }

    pub fn get(&mut self, key: &Key) -> Option<CachedFrame> {
        self.frames.get(key).cloned()
    }

    pub fn put(
        &mut self,
        key: Key,
        source: &Target,
        quads: Vec<LayerQuad>,
        representation: pm_protocol::frame::Representation,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let bytes = source.width() as usize
            * source.height() as usize
            * if source.format() == wgpu::TextureFormat::Rgba32Float {
                16
            } else {
                8
            };
        if bytes > self.frames.stats().budget || self.frames.stats().budget == 0 {
            return;
        }
        let copy = Target::new(
            device,
            "cached preview",
            source.width(),
            source.height(),
            source.format(),
            pm_gpu::target::working_usage(),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cache preview"),
        });
        copy_into(&mut encoder, source, &copy);
        queue.submit([encoder.finish()]);
        self.frames.insert(
            key,
            CachedFrame {
                color: copy,
                quads,
                representation,
            },
            bytes,
        );
    }
}

pub(super) fn copy_into(encoder: &mut wgpu::CommandEncoder, source: &Target, target: &Target) {
    encoder.copy_texture_to_texture(
        source.texture.as_image_copy(),
        target.texture.as_image_copy(),
        wgpu::Extent3d {
            width: source.width(),
            height: source.height(),
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod invariant_tests {
    use super::playback_invariant;

    fn root(json: &str) -> pm_model::Node {
        pm_model::Document::new(Some(pm_model::json::parse(json).unwrap()), 1)
            .root()
            .unwrap()
            .clone()
    }

    #[test]
    fn only_documents_without_blur_or_level2_layers_share_playing_and_paused_frames() {
        assert!(playback_invariant(&root(r#"{"layers":[{"type":"shape","mblur":false}],"comps":{"c":{"layers":[{"type":"text"}]}}}"#)));
        assert!(!playback_invariant(&root(r#"{"layers":[{"type":"shape","mblur":true}]}"#)));
        assert!(!playback_invariant(&root(r#"{"layers":[],"comps":{"c":{"layers":[{"type":"image","compat":2}]}}}"#)));
        assert!(!playback_invariant(&root(r#"{"layers":[{"type":"group","mblur":{"v":0,"kf":[]}}]}"#)));
    }
}
