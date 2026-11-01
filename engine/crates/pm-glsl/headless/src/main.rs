//! Disposable Linux/Vulkan draw regression, independent of the macOS renderer.
//! Build pm-glsl-compiler first; set POWERMOVE_GLSL_COMPILER to that binary.
//! cargo run --locked --manifest-path crates/pm-glsl/headless/Cargo.toml
//! Native driver compilation, submission and readback all run in a child.

use pm_glsl::reflect::{Reflection, UniformType};
use std::{
    borrow::Cow,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const WIDTH: u32 = 8;
const HEIGHT: u32 = 8;

fn main() {
    let coverage = std::env::args().any(|a| a == "--coverage");
    if std::env::args().any(|a| a == "--draw-worker") {
        pollster::block_on(draws(coverage));
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.arg("--draw-worker");
    if coverage {
        command.arg("--coverage");
    }
    let mut child = command.stdin(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(if coverage { 180 } else { 60 });
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "headless draw worker failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("headless draw deadline; worker killed and reaped");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn uniform_data(reflection: &Reflection, index: i32) -> Vec<u8> {
    let Some(block) = &reflection.block else {
        return Vec::new();
    };
    let mut data = vec![0; block.size as usize];
    for member in &block.members {
        let values: Vec<f32> = match member.name.as_str() {
            "u_m" => vec![WIDTH as f32, 0., 0., 0., HEIGHT as f32, 0., 0., 0., 1.],
            "u_res" => vec![WIDTH as f32, HEIGHT as f32],
            "u_uv" => vec![0., 0., 1., 1.],
            "u_pxOrigin" => vec![0., 0.],
            "u_i" => {
                data[member.offset as usize..member.offset as usize + 4]
                    .copy_from_slice(&index.to_le_bytes());
                continue;
            }
            _ => continue,
        };
        for (i, value) in values.iter().enumerate() {
            let relative = match member.ty {
                UniformType::Matrix { rows, stride, .. } => {
                    (i as u32 / rows) * stride + (i as u32 % rows) * 4
                }
                _ => i as u32 * 4,
            };
            let offset = (member.offset + relative) as usize;
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    data
}

async fn draws(coverage: bool) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        })
        .await
        .expect("Vulkan software adapter required; this check never silently skips draws");
    eprintln!("Headless adapter: {}", adapter.get_info().name);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .unwrap();
    let sources = [
        ("coordinates", "o=vec4(v_uv, u_res.x/8., 1.);"),
        (
            "derivatives",
            "o=vec4(dFdx(v_uv.x)*8.,-dFdy(v_uv.y)*8.,1.,1.);",
        ),
        ("origin", "o=vec4(gl_FragCoord.xy/u_res,0.,1.);"),
        (
            "finite-loop",
            "float x=0.;for(int j=0;j<4;j++){x+=0.25;}o=vec4(x);",
        ),
        (
            "index-bounds",
            "float a[3];a[0]=0.25;a[1]=0.5;a[2]=0.75;vec3 v=vec3(0.25,0.5,0.75);o=vec4(a[u_i],v[u_i],float(uint(u_i)+1u==0u),1.);",
        ),
        (
            "division",
            "int d=u_i; o=vec4(float(4/d),float((-2147483647-1)/d),float(uint(4)/uint(d)),1.);",
        ),
        (
            "sampler-array",
            "o=texture(u_tex[0],v_uv)+texture(u_tex[1],v_uv);o.a=1.;",
        ),
        (
            "image-bounds",
            "o=texelFetch(u_tex[0],ivec2(u_i),0);o.a=1.;",
        ),
    ];
    let mut count = 0;
    for (name, body) in sources {
        let source = format!(
            "#version 300 es\nprecision highp float;in vec2 v_uv;uniform vec2 u_res;uniform int u_i;uniform sampler2D u_tex[2];out vec4 o;void main(){{{body}}}"
        );
        let program = pm_glsl::translate(&source, 0).unwrap();
        for index in [-2147483648, -1, 0, 1, 2, 2147483647] {
            let pixels = draw(&device, &queue, &program, index).await;
            assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let p = &pixels[((y * WIDTH + x) * 4) as usize..][..4];
                    assert_eq!(p[3], 255, "{name}: draw did not cover the target");
                    if matches!(name, "coordinates" | "origin") {
                        // Engine targets retain GL row order (S3.md, Origin):
                        // raw row zero is the authored bottom, not the final
                        // top-down display encoding. Test that convention
                        // explicitly rather than changing production geometry.
                        let py = if name == "coordinates" {
                            HEIGHT as f32 - y as f32 - 0.5
                        } else {
                            y as f32 + 0.5
                        };
                        let expected = [
                            ((x as f32 + 0.5) / WIDTH as f32 * 255.).round() as u8,
                            (py / HEIGHT as f32 * 255.).round() as u8,
                            if name == "coordinates" { 255 } else { 0 },
                            255,
                        ];
                        for channel in 0..4 {
                            assert!(
                                p[channel].abs_diff(expected[channel]) <= 1,
                                "coordinates/u_res {x},{y}: {p:?} != {expected:?}"
                            );
                        }
                    } else if matches!(name, "derivatives" | "finite-loop") {
                        assert_eq!(p, &[255; 4], "{name}: {p:?}");
                    } else if name == "sampler-array" {
                        assert_eq!(p, &[96, 96, 96, 255]);
                    } else if name == "image-bounds" {
                        // Robust texelFetch returns zero or a valid texel.
                        for value in &p[..3] {
                            assert!([0, 32].contains(value), "{p:?}");
                        }
                        if index == 0 {
                            assert_eq!(p, &[32, 32, 32, 255]);
                        }
                    } else if name == "index-bounds" {
                        // OOB results are language-undefined. Robust access must
                        // read only this initialized array/vector or zero.
                        for value in &p[..2] {
                            assert!(
                                [0, 64, 128, 191].contains(value),
                                "unexpected indexed value {p:?}"
                            );
                        }
                        if (0..3).contains(&index) {
                            assert_eq!(p[0], [64, 128, 191][index as usize]);
                        }
                        assert_eq!(p[2], if index == -1 { 255 } else { 0 });
                    }
                }
            }
            count += 1;
        }
    }
    eprintln!(
        "PASS: {count} headless GLSL draws/readbacks; coordinates/u_res, derivatives, finite loops, negative/large/wrapped indices, zero/overflow division, reflected sampler arrays and image bounds"
    );
    if coverage {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/probes.json")).unwrap();
        let mut drawn = 0;
        let mut refused = 0;
        let mut integer_outputs = 0;
        for probe in fixture["probes"].as_array().unwrap() {
            let source = probe["template"]
                .as_str()
                .unwrap()
                .replace("@SHOW@", fixture["show"].as_str().unwrap())
                .replace("@PRELUDE_TEX@", fixture["preludeTex"].as_str().unwrap())
                .replace("@PRELUDE_UV@", fixture["preludeUv"].as_str().unwrap());
            let program = match pm_glsl::translate(&source, 0) {
                Ok(program) => program,
                Err(f) => {
                    assert!(
                        matches!(
                            f.stage,
                            pm_glsl::FailureStage::Limits
                                | pm_glsl::FailureStage::EsValidate
                                | pm_glsl::FailureStage::Bind
                        ),
                        "unexpected translation failure: {f}"
                    );
                    refused += 1;
                    continue;
                }
            };
            if probe["id"].as_str().unwrap().ends_with("output-ivec4") {
                // The recorded WebGL draw error/Metal refusal: integer
                // fragment output is incompatible with a float target.
                integer_outputs += 1;
                continue;
            }
            assert_eq!(
                draw(&device, &queue, &program, 0).await.len(),
                (WIDTH * HEIGHT * 4) as usize
            );
            drawn += 1;
        }
        assert_eq!(
            (drawn, refused, integer_outputs),
            (264, 129, 1),
            "coverage verdict changed"
        );
        eprintln!(
            "PASS: {drawn} coverage-probe draws/readbacks, {refused} expected translation refusals, {integer_outputs} recorded integer-output incompatibility"
        );
    }
}

async fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    program: &pm_glsl::Program,
    index: i32,
) -> Vec<u8> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let modules: Vec<_> = [&program.fragment, &program.vertex]
        .iter()
        .map(|stage| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Naga(Cow::Owned(stage.module.clone())),
            })
        })
        .collect();
    let mut layouts = Vec::new();
    let mut groups = Vec::new();
    for stage in [&program.fragment, &program.vertex] {
        let block = stage.reflection.block.as_ref();
        let mut entries: Vec<_> = block
            .iter()
            .map(|block| wgpu::BindGroupLayoutEntry {
                binding: block.binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        for s in &stage.reflection.samplers {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: s.binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: s.binding + pm_glsl::SAMPLER_BINDING_OFFSET,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let data = uniform_data(&stage.reflection, index);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: data.len().max(16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !data.is_empty() {
            queue.write_buffer(&buffer, 0, &data);
        }
        let mut entries: Vec<_> = block
            .iter()
            .map(|b| wgpu::BindGroupEntry {
                binding: b.binding,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let textures: Vec<_> = stage
            .reflection
            .samplers
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let v = if i == 0 { 32 } else { 64 };
                queue.write_texture(
                    texture.as_image_copy(),
                    &[v, v, v, 255],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                texture.create_view(&Default::default())
            })
            .collect();
        let sampler = device.create_sampler(&Default::default());
        for (s, view) in stage.reflection.samplers.iter().zip(&textures) {
            entries.push(wgpu::BindGroupEntry {
                binding: s.binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: s.binding + pm_glsl::SAMPLER_BINDING_OFFSET,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &entries,
        });
        layouts.push(layout);
        groups.push(group);
    }
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &modules[1],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &modules[0],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (256 * HEIGHT) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        for (i, group) in groups.iter().enumerate() {
            pass.set_bind_group(i as u32, group, &[]);
        }
        pass.draw(0..4, 0..1);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    output.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    assert!(scope.pop().await.is_none());
    let mapped = output.get_mapped_range(..).unwrap();
    mapped
        .chunks_exact(256)
        .flat_map(|row| row[..(WIDTH * 4) as usize].iter().copied())
        .collect()
}
