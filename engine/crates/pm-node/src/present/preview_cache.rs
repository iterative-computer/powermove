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
    frames: MemoryCache<Key, CachedFrame>,
}

impl PreviewCache {
    pub fn new() -> Self {
        Self {
            namespace: None,
            frames: MemoryCache::new(0, 16_384),
        }
    }

    pub fn budget(&mut self, bytes: usize) {
        self.frames.set_budget(bytes);
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.namespace = None;
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
