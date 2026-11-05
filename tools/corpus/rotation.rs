//! Mac-only default-path pixel qualification. Deliberately fails while the
//! production rotation guard is present; never install an interpretation override.
use anyhow::{Result, ensure};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::path::Path;

fn document(path: &str, w: u32, h: u32) -> (pm_model::Document, MapResolver) {
    let ch = |v| json!({"v":v,"kf":[],"expr":null});
    let v = json!({"id":"synthetic","compId":"C1","w":w,"h":h,"fps":30,"dur":1,"colorDepth":32,"bg":"#000000","comps":{},
        "assets":{"clip":{"id":"clip","kind":"video","w":w,"h":h,"fps":30,"dur":1,"persisted":true}},
        "layers":[{"id":"clip-layer","type":"video","from":0,"dur":1,"on":true,"d":{"asset":"clip","fit":"stretch","w":w,"h":h,"trim":0,"speed":1},
        "p":{"position.x":ch(w as f64/2.),"position.y":ch(h as f64/2.),"scale.x":ch(100.),"scale.y":ch(100.),"opacity":ch(100.)},"fx":[],"masks":[]}]});
    let mut res = MapResolver::default();
    res.paths.insert("clip".into(), path.into());
    (
        pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1),
        res,
    )
}

struct NativePixels(Renderer);
impl crate::rotation_pixels::PixelRenderer for NativePixels {
    fn frame(&mut self, path: &str, (w, h): (u32, u32), export: bool) -> Result<Vec<f32>, String> {
        let (doc, res) = document(path, w, h);
        let mut request = RenderRequest::at_scale(Size::new(w, h), 1., Precision::Float32);
        request.export = export;
        let frame = self
            .0
            .render(&doc, "C1", Ticks::ZERO, &request, &res)
            .map_err(|e| format!("{e:?}"))?;
        if !frame.failures.is_empty() {
            return Err(format!("pixel failures: {:?}", frame.failures));
        }
        self.0.read_rgba(&frame).map_err(|e| format!("{e:?}"))
    }
}
pub fn proof(spec: &Path) -> Result<()> {
    let spec: Value = serde_json::from_slice(&std::fs::read(spec)?)?;
    let turns = spec["turns"].as_u64().unwrap() as u32;
    ensure!((1..=3).contains(&turns), "invalid synthetic rotation");
    crate::rotation_pixels::qualify(
        &mut NativePixels(Renderer::headless()?),
        spec["plain"].as_str().unwrap(),
        spec["rotated"].as_str().unwrap(),
        (64, 48),
        turns,
    )
    .map_err(anyhow::Error::msg)
}
