//! Read-only estimate probe for a saved project. Run under the shared build
//! lock. Extracted media and results must go to a new directory outside Git.
use pm_engine_service::{EngineService, Output, ViewerConfig};
use pm_protocol::common::{Size, Ticks};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) fn run(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    use anyhow::Context;
    anyhow::ensure!(
        args.len() >= 2 && args.len() <= 5,
        "usage: powermove-native --studio3d-estimate-probe PROJECT.pmv NEW_EVIDENCE_DIR [frames=30] [scale=1] [seconds=0]"
    );
    let input = PathBuf::from(&args[0]);
    let requested = PathBuf::from(&args[1]);
    let parent = requested
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .canonicalize()?;
    let evidence = PathBuf::from(std::env::var_os("HOME").context("No home directory")?)
        .join("Developer/pm-shots")
        .canonicalize()?;
    anyhow::ensure!(
        parent.starts_with(&evidence),
        "Evidence belongs under ~/Developer/pm-shots"
    );
    let out = parent.join(
        requested
            .file_name()
            .context("Invalid evidence directory")?,
    );
    anyhow::ensure!(!out.exists(), "evidence directory already exists");
    std::fs::create_dir_all(&out)?;
    let number = |i: usize| {
        args.get(i)
            .map(|v| v.to_str().context("Invalid numeric argument"))
            .transpose()
    };
    let frames = number(2)?.map(str::parse).transpose()?.unwrap_or(30u64);
    let scale = number(3)?.map(str::parse).transpose()?.unwrap_or(1f32);
    let seconds = number(4)?.map(str::parse).transpose()?.unwrap_or(0f64);
    anyhow::ensure!(
        seconds.is_finite() && seconds >= 0.,
        "invalid calibration time"
    );
    let opened = pm_kernel::project_io::open_project(&input, Default::default())?;
    let composition = opened
        .open_comp
        .clone()
        .ok_or_else(|| anyhow::anyhow!("project has no open composition"))?;
    let mut media = pm_engine_service::MediaCatalog::default();
    for (index, entry) in opened.file.media.iter().enumerate() {
        let extension = match entry.media_type.to_string().as_str() {
            "model/obj" | "text/plain" => "obj",
            "model/gltf+json" => "gltf",
            "model/gltf-binary" => "glb",
            "model/fbx" => "fbx",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/vnd.radiance" => "hdr",
            _ => "bin",
        };
        let path = out.join(format!("media-{index}.{extension}"));
        entry.copy_to(&path)?;
        media.originals.insert(entry.id.to_string(), path);
    }
    let snapshot = pm_kernel::DocumentStore::open(opened).snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("Studio saved-project estimate probe")?)?;
    let viewer = service.viewer(ViewerConfig {
        snapshot,
        composition,
        media,
        output: Output {
            width: 64,
            height: 64,
            display: 1,
            headroom: 1.,
            crop: None,
        },
        layer: None,
        rate: pm_time::Rate::FPS_30,
        end: None,
        audio: false,
    })?;
    let stamp = viewer
        .stamp()
        .ok_or_else(|| anyhow::anyhow!("viewer has no scene stamp"))?;
    let time = Ticks::new((seconds * 705600000.).round() as i64)?;
    let receive =
        viewer.estimate_studio_export_scaled(stamp, time, Size::new(1920, 1080), frames, scale)?;
    let start = Instant::now();
    let result = loop {
        match receive.try_recv() {
            Ok(result) => break result.map_err(anyhow::Error::msg),
            Err(async_channel::TryRecvError::Empty)
                if start.elapsed() < Duration::from_secs(180) =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => {
                break Err(anyhow::anyhow!(
                    "estimate did not settle: {error}; status {:?}",
                    viewer.status()
                ));
            }
        }
    };
    viewer.retire();
    let estimate = result?;
    let json = serde_json::to_string_pretty(&estimate)?;
    std::fs::write(out.join("estimate.json"), format!("{json}\n"))?;
    println!("{json}");
    Ok(())
}
