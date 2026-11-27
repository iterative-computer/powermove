//! No Metal types escape these worker traits. Linux exercises the identical
//! scheduling, immutable snapshot and MCP encoding path with explicit doubles.
use crate::{
    Error, Result, error,
    project::{Project, new_destination},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, PartialEq)]
pub struct FrameSpec {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub revision: u64,
}
pub struct Image {
    pub png: Vec<u8>,
    pub revision: u64,
}
pub trait FrameEngine {
    fn frame(&mut self, project: &Project, request: &FrameSpec) -> Result<Image>;
}
pub trait ExportChild {
    /// Return only after successful child completion and a real output exists.
    fn export(&mut self, project: &Project, args: &Value, destination: &Path) -> Result<()>;
}
pub trait Backend: FrameEngine + ExportChild {
    fn is_test_double(&self) -> bool {
        false
    }
}
pub fn backend(name: &str) -> Result<Box<dyn Backend>> {
    if name == "test-double" {
        return Ok(Box::new(TestDouble));
    }
    #[cfg(target_os = "macos")]
    if name == "native" {
        return Ok(Box::new(native::Native::default()));
    }
    Err(Error::unavailable(
        "native rendering/export requires macOS Metal/VideoToolbox; --backend test-double is available for CI only",
    ))
}
/// Frozen MCP time/width semantics; deduplicate after clamping and rounding.
/// Headless has no transport selection, so the default playhead is zero.
pub fn frame_plan(document: &Value, args: &Value, revision: u64) -> Result<Vec<FrameSpec>> {
    pm_mcp::catalog::validate("render_frames", args)?;
    let dur = document["dur"]
        .as_f64()
        .filter(|n| *n > 0.)
        .unwrap_or(10.)
        .max(0.1);
    let fps = document["fps"].as_f64().filter(|n| *n > 0.).unwrap_or(30.);
    let start = document["work"][0].as_f64().unwrap_or(0.);
    let end = document["work"][1].as_f64().unwrap_or(dur);
    let defaults = [
        0.,
        start,
        start + (end - start) * 0.5,
        start.max(end - 1. / fps),
    ];
    let times = args["times"]
        .as_array()
        .filter(|a| !a.is_empty())
        .map(|a| {
            a.iter()
                .map(|n| n.as_f64().expect("validated time"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| defaults.to_vec());
    let width = args["width"].as_u64().unwrap_or(720) as u32;
    let w = document["w"]
        .as_f64()
        .ok_or_else(|| Error::invalid("Missing composition width"))?;
    let h = document["h"]
        .as_f64()
        .ok_or_else(|| Error::invalid("Missing composition height"))?;
    if w <= 0. || h <= 0. {
        return Err(Error::invalid("Composition dimensions must be positive"));
    }
    let height = (width as f64 * h / w).round().max(1.);
    if height > 8192. {
        return Err(Error::invalid(
            "Frame height exceeds 8192; request a smaller width",
        ));
    }
    let mut out: Vec<FrameSpec> = vec![];
    for time in times {
        let time = (time.clamp(0., dur) * 1000.).round() / 1000.;
        if !out.iter().any(|s| s.time == time) {
            out.push(FrameSpec {
                time,
                width,
                height: height as u32,
                revision,
            });
        }
    }
    Ok(out)
}
pub fn render_frames(project: &Project, args: &Value, backend: &mut dyn Backend) -> Result<Value> {
    let revision = project.published()?.revision();
    let plan = frame_plan(&project.document()?, args, revision)?;
    let mut images = vec![];
    for request in &plan {
        let image = backend.frame(project, request)?;
        if image.revision != revision {
            return Err(Error::conflict(
                "Engine returned a frame for a different revision",
            ));
        }
        if image.png.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
            return Err(error("render_error", "Backend did not return PNG bytes"));
        }
        images
            .push(json!({"type":"image","mimeType":"image/png","data":STANDARD.encode(image.png)}));
    }
    let mut summary = json!({"times":plan.iter().map(|s|s.time).collect::<Vec<_>>(),"width":plan.first().map_or(720,|s|s.width),"rendered":images.len()});
    if backend.is_test_double() {
        summary["testDouble"] = json!(true);
    }
    let mut result = pm_mcp::success(summary);
    result["content"]
        .as_array_mut()
        .expect("MCP content")
        .extend(images);
    Ok(result)
}
pub fn export(
    project: &Project,
    args: &Value,
    destination: &Path,
    backend: &mut dyn Backend,
) -> Result<Value> {
    pm_mcp::catalog::validate("export_video", args)?;
    if args["alpha"] == true && args["format"].as_str().unwrap_or("mp4") != "prores" {
        return Err(Error::invalid("Transparent video requires ProRes"));
    }
    let destination = new_destination(destination)?;
    if backend.is_test_double() && !destination.to_string_lossy().ends_with(".test-export.json") {
        return Err(Error::invalid(
            "Test-double export requires a .test-export.json destination",
        ));
    }
    backend.export(project, args, &destination)?;
    if !destination.is_file() || std::fs::metadata(&destination)?.len() == 0 {
        return Err(error(
            "export_error",
            "Export child did not publish a nonempty output",
        ));
    }
    let p = project.document()?;
    let mut result = json!({"path":destination,"projectId":p["id"],"width":p["w"],"height":p["h"],"fps":p["fps"],"missingMedia":[]});
    if backend.is_test_double() {
        result["testDouble"] = json!(true);
    }
    Ok(result)
}
fn png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![];
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|e| error("render_error", e.to_string()))?
            .write_image_data(rgba)
            .map_err(|e| error("render_error", e.to_string()))?;
    }
    Ok(out)
}
/// Deliberately simple synthetic signal, never a software project renderer.
pub struct TestDouble;
impl FrameEngine for TestDouble {
    fn frame(&mut self, _: &Project, request: &FrameSpec) -> Result<Image> {
        let mut pixels = vec![0; request.width as usize * request.height as usize * 4];
        for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                (i % 251) as u8,
                (request.time * 1000.) as u8,
                request.revision as u8,
                255,
            ]);
        }
        Ok(Image {
            png: png(request.width, request.height, &pixels)?,
            revision: request.revision,
        })
    }
}
impl ExportChild for TestDouble {
    fn export(&mut self, project: &Project, args: &Value, destination: &Path) -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(destination)?;
        let manifest = json!({"testDouble":true,"encodedVideo":false,"revision":project.published()?.revision(),"projectId":project.document()?["id"],"arguments":args});
        file.write_all(&serde_json::to_vec(&manifest).expect("manifest JSON"))?;
        file.sync_all()?;
        Ok(())
    }
}
impl Backend for TestDouble {
    fn is_test_double(&self) -> bool {
        true
    }
}

#[cfg(target_os = "macos")]
#[path = "native_backend.rs"]
mod native;

#[cfg(target_os = "macos")]
pub fn native_with_fonts(
    name: &str,
    fonts: pm_render::text::host::RenderFonts,
) -> Result<Box<dyn Backend>> {
    if name == "test-double" {
        return backend(name);
    }
    if name != "native" {
        return Err(Error::invalid("Unknown backend"));
    }
    Ok(Box::new(native::Native::with_fonts(fonts)))
}
