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

pub fn proof(spec: &Path) -> Result<()> {
    let spec: Value = serde_json::from_slice(&std::fs::read(spec)?)?;
    let turns = spec["turns"].as_u64().unwrap() as u32;
    ensure!((1..=3).contains(&turns), "invalid synthetic rotation");
    let (w, h) = (64, 48);
    let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let mut renderer = Renderer::headless()?;
    let (base, base_res) = document(spec["plain"].as_str().unwrap(), w, h);
    let (rotated, rotated_res) = document(spec["rotated"].as_str().unwrap(), ow, oh);
    for export in [false, true] {
        let mut render = |doc, res: &MapResolver, width, height| -> Result<Vec<f32>> {
            let mut request =
                RenderRequest::at_scale(Size::new(width, height), 1., Precision::Float32);
            request.export = export;
            let frame = renderer.render(doc, "C1", Ticks::ZERO, &request, res)?;
            ensure!(
                frame.failures.is_empty(),
                "pixel failures: {:?}",
                frame.failures
            );
            Ok(renderer.read_rgba(&frame)?)
        };
        let plain = render(&base, &base_res, w, h)?;
        ensure!(
            plain.chunks_exact(4).any(|p| p[0] != p[1]),
            "blank rotation witness"
        );
        let ours = render(&rotated, &rotated_res, ow, oh)?;
        for y in 0..oh {
            for x in 0..ow {
                let (u, v) = match turns {
                    1 => (w - 1 - y, x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (y, h - 1 - x),
                };
                let a = ((y * ow + x) * 4) as usize;
                let b = ((v * w + u) * 4) as usize;
                ensure!(
                    ours[a..a + 4] == plain[b..b + 4],
                    "rotation pixel mismatch ({x},{y}), export={export}"
                );
            }
        }
    }
    Ok(())
}
