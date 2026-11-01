use pm_promedia::{cache::*, cache_frame::*, render::*, scopes::*};
use std::io;
use tempfile::TempDir;
fn request() -> FrameRequest {
    FrameRequest {
        identity: GraphIdentity {
            graph: [1; 32],
            media: [2; 32],
            renderer_version: "synthetic-engine-v1".into(),
            encoding: "bt709-code".into(),
        },
        ticks: 42,
        width: 2,
        height: 1,
    }
}
struct Engine {
    identity: GraphIdentity,
    renders: usize,
    stale: bool,
    wrong: bool,
    pixels: Vec<[f32; 4]>,
}
impl Engine {
    fn new(r: &FrameRequest) -> Self {
        Self {
            identity: r.identity.clone(),
            renders: 0,
            stale: false,
            wrong: false,
            pixels: vec![[0.25, 0.5, 0.75, 0.5], [1.25, -0.25, 0.0, 0.0]],
        }
    }
}
impl FrameSource for Engine {
    fn identity(&self) -> GraphIdentity {
        self.identity.clone()
    }
    fn render(&mut self, r: &FrameRequest) -> io::Result<EngineFrame> {
        self.renders += 1;
        if self.stale {
            self.identity.graph[0] ^= 1;
        }
        let mut r = r.clone();
        if self.wrong {
            r.ticks += 1;
        }
        Ok(EngineFrame {
            request: r,
            rgba: self.pixels.clone(),
        })
    }
}
fn lut() -> LutTransform {
    LutTransform::parse("LUT_1D_SIZE 2\n0 0 0\n2 2 2\n", "bt709-code".into()).unwrap()
}
fn scopes(stage: AnalysisStage) -> Option<ScopeRequest> {
    Some(ScopeRequest {
        stage,
        config: ScopeConfig {
            columns: 2,
            levels: 5,
            vector_bins: 3,
            min: 0.0,
            max: 2.0,
            matrix: Matrix::Bt709,
        },
    })
}
#[test]
fn engine_frames_flow_through_lut_cache_and_scopes_on_miss_and_hit() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let mut source = Engine::new(&r);
    let work = RenderWork::new(r.clone(), Some(lut())).unwrap();
    let result = work
        .run(&mut source, &cache, scopes(AnalysisStage::AfterLut))
        .unwrap();
    assert!(!result.cache_hit);
    assert_eq!(result.frame.rgba[0], [0.5, 1.0, 1.5, 0.5]);
    assert_eq!(result.frame.rgba[1], [2.0, 0.0, 0.0, 0.0]);
    let scope = result.scopes.unwrap();
    assert_eq!(scope.pixels, 2);
    assert_eq!(scope.histogram[0].bins, vec![0, 1, 0, 0, 1]);
    let again = work
        .run(&mut source, &cache, scopes(AnalysisStage::AfterLut))
        .unwrap();
    assert!(again.cache_hit);
    assert_eq!(again.frame, result.frame);
    assert_eq!(again.scopes.unwrap(), scope);
    assert_eq!(source.renders, 1);
}
#[test]
fn scopes_before_lut_use_engine_input_even_when_output_is_cached() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let mut source = Engine::new(&r);
    let work = RenderWork::new(r, Some(lut())).unwrap();
    work.run(&mut source, &cache, None).unwrap();
    let result = work
        .run(&mut source, &cache, scopes(AnalysisStage::BeforeLut))
        .unwrap();
    assert!(result.cache_hit);
    assert_eq!(source.renders, 2);
    assert_eq!(
        result.scopes.unwrap().histogram[0].bins,
        vec![0, 1, 0, 1, 0]
    );
}
#[test]
fn graph_media_colour_renderer_and_lut_changes_invalidate_without_losing_pins() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let old = RenderWork::new(r.clone(), None).unwrap();
    let mut source = Engine::new(&r);
    old.run(&mut source, &cache, None).unwrap();
    let key = old.key().unwrap();
    cache.keep_rendered(&key, true).unwrap();
    let mut changes = Vec::new();
    let mut changed = r.clone();
    changed.identity.graph[0] ^= 1;
    changes.push(changed);
    let mut changed = r.clone();
    changed.identity.media[0] ^= 1;
    changes.push(changed);
    let mut changed = r.clone();
    changed.identity.encoding = "bt2020-code".into();
    changes.push(changed);
    let mut changed = r.clone();
    changed.identity.renderer_version = "v2".into();
    changes.push(changed);
    let mut changed = r.clone();
    changed.ticks += 1;
    changes.push(changed);
    let mut changed = r.clone();
    changed.width = 1;
    changes.push(changed);
    for changed in changes {
        let work = RenderWork::new(changed, None).unwrap();
        assert_eq!(
            cache.state(&work.key().unwrap(), false).unwrap(),
            BarState::Missing
        );
    }
    assert_eq!(
        cache
            .state(
                &RenderWork::new(r.clone(), Some(lut()))
                    .unwrap()
                    .key()
                    .unwrap(),
                false
            )
            .unwrap(),
        BarState::Missing
    );
    assert_eq!(cache.state(&key, false).unwrap(), BarState::Kept);
    source.identity.graph[0] ^= 1;
    assert!(old.run(&mut source, &cache, None).is_err());
    assert!(cache.get(&key).unwrap().is_some());
}
#[test]
fn publication_during_engine_render_does_not_fill_cache() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let mut source = Engine::new(&r);
    source.stale = true;
    let work = RenderWork::new(r, None).unwrap();
    assert!(work.run(&mut source, &cache, None).is_err());
    assert!(cache.get(&work.key().unwrap()).unwrap().is_none());
}
#[test]
fn wrong_frame_nonfinite_alpha_and_scope_configuration_are_refused() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let work = RenderWork::new(r.clone(), None).unwrap();
    let mut source = Engine::new(&r);
    source.wrong = true;
    assert!(work.run(&mut source, &cache, None).is_err());
    for pixel in [
        [f32::NAN, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, -1.0],
        [0.0, 0.0, 0.0, 1.1],
    ] {
        let mut source = Engine::new(&r);
        source.pixels[0] = pixel;
        assert!(work.run(&mut source, &cache, None).is_err());
    }
    let mut cfg = scopes(AnalysisStage::AfterLut).unwrap();
    cfg.config.columns = 0;
    assert!(work.run(&mut Engine::new(&r), &cache, Some(cfg)).is_err());
    assert!(cache.get(&work.key().unwrap()).unwrap().is_none());
}
#[test]
fn malformed_and_mismatched_luts_refuse_and_extended_rgb_survives() {
    assert!(LutTransform::parse("invalid", "bt709-code".into()).is_err());
    let mut r = request();
    r.identity.encoding = "bt2020-code".into();
    assert!(RenderWork::new(r, Some(lut())).is_err());
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let mut source = Engine::new(&r);
    let result = RenderWork::new(r, None)
        .unwrap()
        .run(&mut source, &cache, None)
        .unwrap();
    assert_eq!(result.frame.rgba[1], [1.25, -0.25, 0.0, 0.0]);
}
#[test]
fn cached_payload_requires_exact_shape_and_finite_samples() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let work = RenderWork::new(r.clone(), None).unwrap();
    cache.put(&work.key().unwrap(), b"bad shape").unwrap();
    assert!(work.run(&mut Engine::new(&r), &cache, None).is_err());
}
#[test]
fn lut_f32_overflow_is_never_cached() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let lut = LutTransform::parse(
        "LUT_1D_SIZE 2\n1e100 0 0\n1e100 1 1\n",
        r.identity.encoding.clone(),
    )
    .unwrap();
    let work = RenderWork::new(r.clone(), Some(lut)).unwrap();
    assert!(work.run(&mut Engine::new(&r), &cache, None).is_err());
    assert!(cache.get(&work.key().unwrap()).unwrap().is_none());
}

#[test]
fn actor_receipt_rejects_late_graph_and_lut_results() {
    let dir = TempDir::new().unwrap();
    let cache = RenderCache::open(dir.path(), 4096).unwrap();
    let r = request();
    let work = RenderWork::new(r.clone(), None).unwrap();
    let result = work.run(&mut Engine::new(&r), &cache, None).unwrap();
    let changed_lut = RenderWork::new(r.clone(), Some(lut())).unwrap();
    assert!(changed_lut.accept(result, &r.identity).is_err());
    let result = work.run(&mut Engine::new(&r), &cache, None).unwrap();
    let mut changed = r.identity.clone();
    changed.media[0] ^= 1;
    assert!(work.accept(result, &changed).is_err());
    let result = work.run(&mut Engine::new(&r), &cache, None).unwrap();
    assert!(work.accept(result, &r.identity).is_ok());
}
#[test]
fn oversized_or_incomplete_requests_refuse_before_readback() {
    for (width, height) in [(0, 1), (1, 0), (u32::MAX, u32::MAX), (u32::MAX, 1)] {
        let mut r = request();
        r.width = width;
        r.height = height;
        assert!(RenderWork::new(r, None).is_err());
    }
    let mut r = request();
    r.identity.encoding.clear();
    assert!(RenderWork::new(r, None).is_err());
    let mut r = request();
    r.identity.renderer_version.clear();
    assert!(RenderWork::new(r, None).is_err());
}
