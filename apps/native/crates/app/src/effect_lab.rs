//! A private composition drawn by the same compositor and effect planner
//! used by the viewer and exports. Only backgrounds, labels and scoring are
//! prepared here. One GPU renderer serves the entire sweep.
use crate::mcp_services::{Rgba, decode_png, fit, image, png};
use anyhow::{Result, anyhow, bail, ensure};
use pm_mcp::effect_lab::{Backdrop, Context, Request, Shape};
use pm_mcp::services::ToolOutput;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
    definitions::{DefinitionOrigin, DefinitionSnapshot, ParamSpec},
    snapshot::SnapshotPixels,
};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Map, Value, json};

pub(crate) fn render(
    request: &Request,
    context: &Context,
    definition: &DefinitionSnapshot,
    composition_png: Option<&[u8]>,
    stop: &dyn Fn() -> bool,
) -> Result<ToolOutput> {
    let parameter_sets = request
        .parameter_sets
        .iter()
        .map(|overrides| parameters(definition, overrides))
        .collect::<Result<Vec<_>>>()?;
    let size = Size::new(request.width, request.height);
    let shape = Geometry::new(&request.shape, size);
    let reference = request
        .reference_path
        .as_deref()
        .map(|path| read_image(context, path, size).map(|pixels| luma(&pixels)))
        .transpose()?;
    let background = background(request, context, composition_png)?;
    let scratch = tempfile::tempdir()?;
    let mut assets = MapResolver::default();
    for (id, pixels, w, h) in [
        ("backdrop", background.clone(), size.width, size.height),
        (
            "subject",
            foreground(&background, size, shape),
            shape.width,
            shape.height,
        ),
    ] {
        let path = scratch.path().join(format!("{id}.png"));
        std::fs::write(&path, png(&pixels, w, h)?)?;
        assets.paths.insert(id.into(), path);
    }

    let mut renderer = Renderer::headless()?;
    renderer
        .effects_mut()
        .watchdog
        .set_policy(pm_render::effects::watchdog::Policy::Job);
    let mut capture = RenderRequest::sized(size, Precision::Float16);
    capture.export = true;
    let seconds = match request.backdrop {
        Backdrop::Composition { time, .. } => time,
        _ => 0.0,
    };
    let ticks = Ticks::new(pm_time::ticks_from_seconds(seconds)?)?;
    let mut sheet = Sheet::new(size, parameter_sets.len());
    let mut cells = Vec::new();
    for (index, parameters) in parameter_sets.iter().enumerate() {
        ensure!(!stop(), "The effect lab render timed out.");
        let document = document(size, shape, definition, parameters, index as u64)?;
        if index == 0 {
            let contributed = match definition.origin {
                DefinitionOrigin::BuiltIn => &[][..],
                _ => std::slice::from_ref(definition),
            };
            let refused = renderer
                .set_document_effects(&document, contributed)
                .map_err(|e| anyhow!(e))?;
            ensure!(
                refused.is_empty(),
                "{}",
                refused
                    .iter()
                    .map(|e| e.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            ensure!(
                renderer.missing_effects().is_empty(),
                "The selected effect is unavailable."
            );
        }
        let frame = renderer.render(&document, "effect-lab-comp", ticks, &capture, &assets)?;
        ensure!(
            frame.failures.is_empty(),
            "Effect render failed: {:?}",
            frame.failures
        );
        renderer.wait_idle();
        if let Some(reason) = renderer.watchdog_stopped() {
            bail!("{reason}");
        }
        let rgba = renderer.read_rgba_reusing(&frame)?;
        let pixels = pm_export::raster::encode_rgba8_f32(&rgba, SnapshotPixels::Straight);
        renderer.give_rgba(rgba);
        renderer.recycle(frame);
        let scores = reference
            .as_ref()
            .map(|reference| similarity(reference, &luma(&pixels), size, shape));
        sheet.add(index, &pixels, scores);
        let mut cell = json!({"index":index+1, "parameters":parameters});
        if let Some([whole, edge]) = scores {
            cell["scores"] = json!({"wholeImage":whole,"edgeBand":edge});
        }
        cells.push(cell);
    }
    ensure!(!stop(), "The effect lab render timed out.");
    let mut metadata = json!({"effectId":request.effect_id,"width":size.width,"height":size.height,
        "columns":sheet.columns,"rows":sheet.rows,"labelHeight":LABEL_HEIGHT,"cells":cells});
    if reference.is_some() {
        metadata["metric"] = json!("SSIM");
    }
    let png = image(&sheet.canvas, sheet.width(), sheet.height()).map_err(|e| anyhow!(e))?;
    Ok(ToolOutput {
        value: metadata,
        images: vec![png],
    })
}

fn parameters(
    definition: &DefinitionSnapshot,
    overrides: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    for key in overrides.keys() {
        ensure!(
            definition.params.iter().any(|param| param.key() == key),
            "Unknown parameter {key} on {}.",
            definition.id.as_str()
        );
    }
    definition
        .params
        .iter()
        .map(|param| {
            let key = param.key();
            let value = match (param, overrides.get(key)) {
                (ParamSpec::Number { default, .. }, None) => json!(default),
                (ParamSpec::Color { default, .. }, None) => json!(default),
                (ParamSpec::Toggle { default, .. }, None) => json!(default),
                (ParamSpec::GradientStops { default, .. }, None) => json!(default),
                (ParamSpec::Number { min, max, .. }, Some(v))
                    if v.as_f64().is_some_and(|n| (*min..=*max).contains(&n)) =>
                {
                    v.clone()
                }
                (ParamSpec::Color { .. }, Some(v))
                    if v.as_str().is_some_and(|s| color(s).is_ok()) =>
                {
                    v.clone()
                }
                (ParamSpec::Toggle { .. }, Some(v)) if v.is_boolean() => v.clone(),
                // Effect snapshots normalize gradient stops to straight RGBA.
                (ParamSpec::GradientStops { .. }, Some(v)) => {
                    let stops: Vec<pm_protocol::definitions::GradientStop> =
                        serde_json::from_value(v.clone())
                            .map_err(|_| anyhow!("Invalid gradient stops for {key}."))?;
                    ensure!(
                        (2..=16).contains(&stops.len())
                            && stops.iter().all(|stop| (0.0..=1.0).contains(&stop.position)
                                && stop.color.iter().all(|v| (0.0..=1.0).contains(v))),
                        "Invalid gradient stops for {key}."
                    );
                    v.clone()
                }
                _ => bail!(
                    "Invalid value for parameter {key} on {}.",
                    definition.id.as_str()
                ),
            };
            Ok((key.to_owned(), value))
        })
        .collect()
}

fn color(s: &str) -> Result<[u8; 4]> {
    let hex = s
        .strip_prefix('#')
        .filter(|s| s.len() == 6 && s.is_ascii())
        .ok_or_else(|| anyhow!("Use a #RRGGBB color."))?;
    let value = u32::from_str_radix(hex, 16)?;
    Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8, 255])
}

fn read_image(context: &Context, path: &std::path::Path, size: Size) -> Result<Vec<u8>> {
    let bytes = context.read_image(path)?;
    let mut reader =
        ::image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .map_err(|error| anyhow!("Could not decode the workspace image ({error})."))?
        .into_rgba8();
    Ok(::image::imageops::resize(
        &pixels,
        size.width,
        size.height,
        ::image::imageops::FilterType::Triangle,
    )
    .into_raw())
}

fn background(
    request: &Request,
    context: &Context,
    composition_png: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let (w, h) = (request.width, request.height);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    let mut fill = |pixel: &dyn Fn(u32, u32) -> [u8; 4]| {
        for y in 0..h {
            for x in 0..w {
                pixels.extend(pixel(x, y));
            }
        }
    };
    match &request.backdrop {
        Backdrop::Checker { tile_size, colors } => {
            let colors = [color(&colors[0])?, color(&colors[1])?];
            fill(&|x, y| colors[((x / tile_size + y / tile_size) % 2) as usize]);
        }
        Backdrop::Solid { color: hex } => {
            let pixel = color(hex)?;
            fill(&|_, _| pixel);
        }
        Backdrop::Gradient { colors, angle } => {
            let [start, end] = [color(&colors[0])?, color(&colors[1])?];
            let (dy, dx) = angle.unwrap_or(0.).to_radians().sin_cos();
            let span = dx.abs() * w as f64 + dy.abs() * h as f64;
            fill(&|x, y| {
                let t = ((dx * (x as f64 + 0.5 - w as f64 / 2.)
                    + dy * (y as f64 + 0.5 - h as f64 / 2.))
                    / span
                    + 0.5)
                    .clamp(0., 1.);
                std::array::from_fn(|i| {
                    (start[i] as f64 * (1. - t) + end[i] as f64 * t).round() as u8
                })
            });
        }
        Backdrop::Image { path } => return read_image(context, path, Size::new(w, h)),
        Backdrop::Composition { .. } => {
            let frame: Rgba = decode_png(
                composition_png.ok_or_else(|| anyhow!("The composition frame is missing."))?,
            )?;
            return Ok(fit(&frame, w, h));
        }
    }
    Ok(pixels)
}

#[derive(Clone, Copy)]
struct Geometry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    radius: f64,
}

impl Geometry {
    fn new(shape: &Shape, size: Size) -> Self {
        let (width, height, radius) = match *shape {
            Shape::Full => (size.width, size.height, 0.),
            Shape::Rect {
                width,
                height,
                corner_radius,
            } => (width, height, corner_radius.unwrap_or(0.)),
            Shape::Capsule { width, height } => (width, height, width.min(height) as f64 / 2.),
            Shape::Circle { diameter } => (diameter, diameter, diameter as f64 / 2.),
        };
        Self {
            x: (size.width - width) / 2,
            y: (size.height - height) / 2,
            width,
            height,
            radius: radius.min(width.min(height) as f64 / 2.),
        }
    }

    fn distance(self, x: u32, y: u32) -> f64 {
        let half_w = self.width as f64 / 2.;
        let half_h = self.height as f64 / 2.;
        let dx = (x as f64 + 0.5 - self.x as f64 - half_w).abs() - half_w + self.radius;
        let dy = (y as f64 + 0.5 - self.y as f64 - half_h).abs() - half_h + self.radius;
        dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - self.radius
    }
}

fn foreground(background: &[u8], size: Size, shape: Geometry) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((shape.width * shape.height * 4) as usize);
    for y in shape.y..shape.y + shape.height {
        for x in shape.x..shape.x + shape.width {
            let at = ((y * size.width + x) * 4) as usize;
            pixels.extend_from_slice(&background[at..at + 3]);
            let coverage = (0.5 - shape.distance(x, y)).clamp(0., 1.);
            pixels.push((background[at + 3] as f64 * coverage).round() as u8);
        }
    }
    pixels
}

fn document(
    size: Size,
    shape: Geometry,
    definition: &DefinitionSnapshot,
    params: &Map<String, Value>,
    revision: u64,
) -> Result<pm_model::Document> {
    let channel = |v: Value| json!({"v":v,"kf":[],"expr":null});
    let mut assets = Map::new();
    let mut layer = |id: &str, x: u32, y: u32, w: u32, h: u32| {
        assets.insert(
            id.into(),
            json!({"id":id,"name":id,"kind":"image","format":"png","w":w,"h":h,"persisted":true}),
        );
        json!({"id":id,"type":"image","name":id,"from":0,"dur":86401,"on":true,"blend":"normal",
            "p":{"position.x":channel(json!(x as f64+w as f64/2.)),"position.y":channel(json!(y as f64+h as f64/2.)),
                 "scale.x":channel(json!(100)),"scale.y":channel(json!(100)),"opacity":channel(json!(100))},
            "d":{"asset":id,"fit":"stretch","w":w,"h":h},"fx":[],"masks":[]})
    };
    let mut subject = layer("subject", shape.x, shape.y, shape.width, shape.height);
    let backdrop = layer("backdrop", 0, 0, size.width, size.height);
    let channels = params
        .iter()
        .map(|(key, value)| (key.clone(), channel(value.clone())))
        .collect::<Map<_, _>>();
    let mut effect = json!({"id":"sweep","type":definition.id.as_str(),"on":true,"p":channels});
    if matches!(definition.origin, DefinitionOrigin::BuiltIn) {
        effect["version"] = json!(definition.version);
    }
    subject["fx"] = json!([effect]);
    let raw = json!({"id":"effect-lab","name":"Effect Lab","w":size.width,"h":size.height,
        "fps":30,"dur":86401,"compId":"effect-lab-comp","compName":"Effect Lab","bg":"#000000",
        "backgroundFill":{"type":"none"},"layers":[subject,backdrop],"assets":assets,"comps":{}});
    let value = pm_model::json::parse(&raw.to_string()).map_err(|e| anyhow!("{e:?}"))?;
    let mut document = pm_model::Document::new(Some(value), revision);
    // This is a new composition. Resolve version-one effects against the
    // current catalog when their ID also existed in the older 1.2.0 file format.
    document.set_saving_app_version(Some("1.2.1".into()));
    Ok(document)
}

fn luma(rgba: &[u8]) -> Vec<f64> {
    rgba.chunks_exact(4)
        .map(|p| {
            let alpha = f64::from(p[3]) / 255.;
            (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) * alpha
                / 255.
        })
        .collect()
}

#[derive(Default)]
struct Moments {
    count: u32,
    a: f64,
    b: f64,
    aa: f64,
    bb: f64,
    ab: f64,
}

impl Moments {
    fn add(&mut self, a: f64, b: f64) {
        self.count += 1;
        self.a += a;
        self.b += b;
        self.aa += a * a;
        self.bb += b * b;
        self.ab += a * b;
    }

    fn score(&self) -> f64 {
        let n = self.count as f64;
        let (a, b) = (self.a / n, self.b / n);
        let (va, vb) = ((self.aa / n - a * a).max(0.), (self.bb / n - b * b).max(0.));
        let covariance = self.ab / n - a * b;
        ((2. * a * b + 0.0001) * (2. * covariance + 0.0009)
            / ((a * a + b * b + 0.0001) * (va + vb + 0.0009)))
            .clamp(-1., 1.)
    }
}

/// Mean SSIM on 8-by-8 luma windows. Edge windows count only pixels near
/// the shape outline, even when the shape sits away from the frame border.
fn similarity(a: &[f64], b: &[f64], size: Size, shape: Geometry) -> [f64; 2] {
    let band = (shape.width.min(shape.height) as f64 * 0.03).max(2.);
    let mut scores = [0.; 2];
    let mut counts = [0; 2];
    for top in (0..size.height).step_by(8) {
        for left in (0..size.width).step_by(8) {
            let mut stats = [Moments::default(), Moments::default()];
            for y in top..(top + 8).min(size.height) {
                for x in left..(left + 8).min(size.width) {
                    let i = (y * size.width + x) as usize;
                    stats[0].add(a[i], b[i]);
                    if shape.distance(x, y).abs() <= band {
                        stats[1].add(a[i], b[i]);
                    }
                }
            }
            for (i, m) in stats.iter().enumerate().filter(|(_, m)| m.count > 0) {
                scores[i] += m.score() * m.count as f64;
                counts[i] += m.count;
            }
        }
    }
    [scores[0] / counts[0] as f64, scores[1] / counts[1] as f64]
}

const LABEL_HEIGHT: u32 = 24;
const GLYPHS: [[u8; 5]; 12] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
    [0, 0, 0, 0, 2],
    [0, 0, 7, 0, 0],
];

struct Sheet {
    canvas: Vec<u8>,
    cell: Size,
    columns: u32,
    rows: u32,
}

impl Sheet {
    fn new(cell: Size, count: usize) -> Self {
        let columns = (count as f64).sqrt().ceil() as u32;
        let rows = (count as u32).div_ceil(columns);
        let mut canvas =
            vec![26; (cell.width * columns * (cell.height + LABEL_HEIGHT) * rows * 4) as usize];
        for alpha in canvas.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Self {
            canvas,
            cell,
            columns,
            rows,
        }
    }

    fn width(&self) -> u32 {
        self.cell.width * self.columns
    }
    fn height(&self) -> u32 {
        (self.cell.height + LABEL_HEIGHT) * self.rows
    }

    fn add(&mut self, index: usize, pixels: &[u8], scores: Option<[f64; 2]>) {
        let x = (index as u32 % self.columns) * self.cell.width;
        let y = (index as u32 / self.columns) * (self.cell.height + LABEL_HEIGHT);
        let width = self.width() as usize;
        let line = self.cell.width as usize * 4;
        for (j, row) in pixels.chunks_exact(line).enumerate() {
            let at = ((y as usize + LABEL_HEIGHT as usize + j) * width + x as usize) * 4;
            self.canvas[at..at + line].copy_from_slice(row);
        }
        let mut label = (index + 1).to_string();
        if let Some([whole, edge]) = scores {
            let scored = format!("{label}  {whole:.3}  {edge:.3}");
            if scored.len() * 8 + 12 <= self.cell.width as usize {
                label = scored;
            }
        }
        for (i, ch) in label.bytes().enumerate() {
            let glyph = match ch {
                b'0'..=b'9' => (ch - b'0') as usize,
                b'.' => 10,
                b'-' => 11,
                _ => continue,
            };
            for (dy, bits) in GLYPHS[glyph].iter().enumerate() {
                for dx in 0..3 {
                    if bits & (4 >> dx) == 0 {
                        continue;
                    }
                    let at =
                        ((y as usize + 7 + dy * 2) * width + x as usize + 8 + i * 8 + dx * 2) * 4;
                    for offset in [0, 4, width * 4, width * 4 + 4] {
                        self.canvas[at + offset..at + offset + 4].copy_from_slice(&[255; 4]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use pm_mcp::{Automation, McpSession};
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Instant,
    };

    struct Fixture {
        session: McpSession,
        workspace: PathBuf,
        stage: PathBuf,
        profile: tempfile::TempDir,
        _actor: pm_kernel::KernelActor,
    }

    impl Fixture {
        fn new() -> Self {
            let profile = tempfile::tempdir().unwrap();
            let path = profile.path().to_owned();
            let actor = pm_kernel::KernelActor::spawn(Default::default());
            let engine = crate::mcp_services::EngineJobs::new(
                actor.handle(),
                std::env::current_exe().unwrap(),
                Default::default(),
                path.clone(),
                Default::default(),
            );
            let mut automation = Automation::new(path.clone(), path.join("exports")).unwrap();
            automation.engine = Some(engine);
            automation.extensions = Some(Arc::new(crate::extension_authoring::AppExtensions::new(
                path,
            )));
            let mut session = McpSession::new(Arc::new(Mutex::new(automation)));
            let started = session.call(
                "start_session",
                json!({"context":"app","access":"editor","prompt":"Sweep a synthetic effect"}),
            );
            assert_eq!(started["isError"], false, "{started}");
            let result: Value =
                serde_json::from_str(started["content"][0]["text"].as_str().unwrap()).unwrap();
            let workspace = result["workspace"].as_str().unwrap().into();
            let stage = result["stagingDirectory"].as_str().unwrap().into();
            Self {
                session,
                workspace,
                stage,
                profile,
                _actor: actor,
            }
        }

        fn render(&mut self, args: Value) -> (Value, Vec<u8>) {
            let result = self.session.call("render_effect_lab", args);
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(result["content"].as_array().unwrap().len(), 2);
            assert_eq!(result["content"][1]["mimeType"], "image/png");
            let bytes = STANDARD
                .decode(result["content"][1]["data"].as_str().unwrap())
                .unwrap();
            (result["structuredContent"].clone(), bytes)
        }
    }

    fn checker() -> Value {
        json!({"type":"checker","colors":["#E02040","#2050DC"],"tileSize":16})
    }

    fn extract_cell(sheet: &Rgba, data: &Value, index: u32) -> Vec<u8> {
        let integer = |key| data[key].as_u64().unwrap() as u32;
        let (w, h, labels, columns) = (
            integer("width"),
            integer("height"),
            integer("labelHeight"),
            integer("columns"),
        );
        let x = index % columns * w;
        let y = index / columns * (h + labels) + labels;
        let mut pixels = Vec::new();
        for row in y..y + h {
            let start = ((row * sheet.1 + x) * 4) as usize;
            pixels.extend_from_slice(&sheet.0[start..start + (w * 4) as usize]);
        }
        pixels
    }

    #[test]
    fn effect_lab_blur_cells_differ_and_identical_reference_ranks_highest() {
        let mut fixture = Fixture::new();
        let mut args = json!({"effectId":"blur","backdrop":checker(),"width":128,"height":128,
            "parameterSets":[{"amount":0},{"amount":6},{"amount":32}]});
        let (metadata, output) = fixture.render(args.clone());
        let sheet = decode_png(&output).unwrap();
        let [first, middle, last] =
            std::array::from_fn(|i| extract_cell(&sheet, &metadata, i as u32));
        let difference =
            |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 8).count();
        assert!(difference(&first, &middle) > 1000);
        assert!(difference(&middle, &last) > 1000);
        fs::write(
            fixture.workspace.join("inputs/reference.png"),
            png(&middle, 128, 128).unwrap(),
        )
        .unwrap();
        args["referencePath"] = json!("inputs/reference.png");
        let (scores, _) = fixture.render(args);
        assert_eq!(scores["metric"], "SSIM");
        for name in ["wholeImage", "edgeBand"] {
            let score = |i: usize| scores["cells"][i]["scores"][name].as_f64().unwrap();
            assert!((score(1) - 1.).abs() < 1e-9);
            assert!(score(1) > score(0) && score(1) > score(2));
        }
    }

    #[test]
    fn effect_lab_samples_the_real_backdrop_from_a_staged_and_installed_effect() {
        let mut fixture = Fixture::new();
        let extension = fixture.stage.join("glass-sample");
        fs::create_dir(&extension).unwrap();
        fs::write(extension.join("manifest.json"),json!({
            "id":"glass-sample","name":"Glass Sample","version":"1.0.0","apiVersion":3,"entry":"index.ts"
        }).to_string()).unwrap();
        fs::write(extension.join("index.ts"),r#"
            export default function activate(api) {
                api.effects.register({
                    id: "glass-swap", label: "Glass Swap", group: "Distort", params: [],
                    backdrop: true,
                    frag: "vec4 under = bd(v_st); float mask = src(v_st).a; o = vec4(under.bgr * mask, mask);"
                });
            }
        "#).unwrap();
        let mut args = json!({"effectId":"glass-swap","extensionId":"glass-sample","backdrop":checker(),
            "shape":{"type":"capsule","width":84,"height":52},"width":128,"height":128,"parameterSets":[{}]});
        let (metadata, output) = fixture.render(args.clone());
        let pixels = extract_cell(&decode_png(&output).unwrap(), &metadata, 0);
        let pixel = |x: usize, y: usize| {
            let start = (y * 128 + x) * 4;
            &pixels[start..start + 4]
        };
        for ((x, y), expected) in [
            ((5, 5), [224_u8, 32, 64, 255]),
            ((57, 57), [64, 32, 224, 255]),
            ((72, 57), [220, 80, 32, 255]),
        ] {
            assert!(
                pixel(x, y)
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 3),
                "{x},{y}: {:?}",
                pixel(x, y)
            );
        }
        let live = fixture.profile.path().join("extensions/glass-sample");
        assert!(!live.exists(), "preview must leave the effect unpublished");
        fs::create_dir_all(&live).unwrap();
        for file in ["manifest.json", "index.ts"] {
            fs::copy(extension.join(file), live.join(file)).unwrap();
        }
        args.as_object_mut().unwrap().remove("extensionId");
        let (_, installed) = fixture.render(args);
        assert_eq!(installed, output);
    }

    #[test]
    fn effect_lab_example_and_twelve_cell_timing() {
        let Some(path) = std::env::var_os("PM_EFFECT_LAB_EXAMPLE") else {
            return;
        };
        let mut fixture = Fixture::new();
        let settings = (0..12).map(|i| json!({"amount":i*4})).collect::<Vec<_>>();
        let start = Instant::now();
        let (_, bytes) = fixture.render(json!({"effectId":"blur","backdrop":checker(),
            "shape":{"type":"capsule","width":416,"height":304},"width":512,"height":512,"parameterSets":settings}));
        println!(
            "12 cells at 512 × 512 in {:.2}s",
            start.elapsed().as_secs_f64()
        );
        fs::write(path, bytes).unwrap();
    }
}
