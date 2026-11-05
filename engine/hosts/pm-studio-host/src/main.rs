//! Headless synthetic Studio export witness. Never discovers user projects.
use pm_studio::{
    extract::{MeshInstance, SceneInput, SceneRequest, SceneSource, Transform},
    service::{Denoise, StudioService, Tier},
};
use std::{path::Path, sync::Arc};

struct Preset {
    name: String,
}
impl SceneSource for Preset {
    fn scene(&self, _: &SceneRequest) -> Result<SceneInput, String> {
        let mut scene = pm_studio::scene::preset(&self.name, "space-gray", 0.)
            .ok_or("unknown Studio preset")?;
        let instances = std::mem::take(&mut scene.objects)
            .into_iter()
            .enumerate()
            .map(|(i, o)| MeshInstance {
                layer: format!("object-{i}"),
                mesh: Arc::new(o.mesh),
                material: o.material,
                world: Transform::default(),
            })
            .collect();
        // The procedural preset's display is a real generated map, not missing pixels.
        let (w, h) = (64, 64);
        let screen = pm_studio::scene::screen_wallpaper(w, h, 0.);
        Ok(SceneInput {
            scene,
            instances,
            materials: None,
            maps: Some(pm_studio::extract::TextureMaps {
                width: w,
                height: h,
                layers: vec![screen],
            }),
            view: Default::default(),
        })
    }
}

fn read(service: &StudioService, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
    let (w, h) = (texture.width(), texture.height());
    let stride =
        (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let device = service.device().ok_or("no Studio device")?;
    let queue = service.queue().ok_or("no Studio queue")?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("studio host export"),
        size: u64::from(stride) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(h),
            },
        },
        texture.size(),
    );
    queue.submit([enc.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    recv.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let map = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| e.to_string())?;
    // PNG stores straight sRGB; compositor textures store premultiplied sRGB.
    let mut out: Vec<u8> = map
        .chunks_exact(stride as usize)
        .flat_map(|r| r[..w as usize * 4].iter().copied())
        .collect();
    for p in out.chunks_exact_mut(4) {
        if p[3] > 0 {
            for c in 0..3 {
                p[c] = (u32::from(p[c]) * 255 / u32::from(p[3])).min(255) as u8;
            }
        }
    }
    Ok(out)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: pm-studio-host PRESET OUTPUT_DIRECTORY WIDTH HEIGHT FRAME_SALT".into());
    }
    let width = args[2].parse::<u32>().map_err(|e| e.to_string())?;
    let height = args[3].parse::<u32>().map_err(|e| e.to_string())?;
    let salt = args[4].parse::<u32>().map_err(|e| e.to_string())?;
    let source = Preset {
        name: args[0].clone(),
    };
    let request = SceneRequest {
        composition: "synthetic-studio".into(),
        revision: 1,
        ticks: i64::from(salt),
        width,
        height,
    };
    let mut service = StudioService::default();
    let result = service.render(
        &source,
        request,
        Tier::Final {
            samples: 8,
            salt,
            denoise: Denoise::Guided,
        },
    )?;
    let dir = Path::new(&args[1]);
    // Refuse an existing destination; never delete or replace another output.
    std::fs::create_dir(dir).map_err(|e| e.to_string())?;
    let mut layers = Vec::new();
    for (i, layer) in result.layers.iter().enumerate() {
        let file = format!("layer-{i:04}.png");
        pm_studio::gpu::write_png(
            &dir.join(&file),
            width,
            height,
            &read(&service, &layer.texture)?,
        )
        .map_err(|e| e.to_string())?;
        layers.push(serde_json::json!({"id":layer.layer,"file":file}));
    }
    let manifest = serde_json::json!({"schema":1,"composition":result.request.composition,"revision":result.request.revision,"ticks":result.request.ticks,"width":width,"height":height,"samples":result.samples,"salt":salt,"adapter":result.adapter,"hardwareRt":result.hardware_rt,"layers":layers,"encoding":"straight sRGB RGBA8 PNG"});
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{manifest}");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
