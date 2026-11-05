//! Frame-aligned viewport depth, IDs and unoccluded selection coverage.
//! Uses the native three.js vertex programs, including skin/morph/instance
//! attributes. The renderer never sends scene geometry back to Chromium.
use super::*;
use crate::scene3d::model::{Material3d, MaterialKind, Materials};
use crate::three::materials::Side;

const DEPTH_FRAGMENT: &str = r#"
void main() {
  uint bits = floatBitsToUint(gl_FragCoord.z);
  gl_FragColor = vec4(float(bits & 2047u) / 2048.0,
    float((bits >> 11u) & 2047u) / 2048.0,
    float(bits >> 22u) / 2048.0, 1.0);
}"#;
const ID_FRAGMENT: &str = "uniform vec3 diffuse; void main(){ gl_FragColor=vec4(diffuse,1.0); }";
pub(super) struct Targets {
    /// Full-precision window depth, before the lossless transport encoding.
    depth: Target,
}
impl Targets {
    fn new(device: &wgpu::Device, w: u32, h: u32) -> Self {
        let depth = Target::new(
            device,
            "chrome full-precision depth",
            w,
            h,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        Self { depth }
    }
}
fn packet(depth: f32) -> wgpu::Color {
    let bits = depth.to_bits();
    wgpu::Color {
        r: f64::from(bits & 2047) / 2048.0,
        g: f64::from((bits >> 11) & 2047) / 2048.0,
        b: f64::from(bits >> 22) / 2048.0,
        a: 1.0,
    }
}
fn override_material(side: Side) -> Material3d {
    Material3d {
        kind: MaterialKind::Basic,
        side,
        ..Material3d::standard_default()
    }
}
fn scene_override(scene: &Scene3d, side: Side) -> Scene3d {
    let mut scene = scene.clone();
    scene.background = None;
    scene.shadows = false;
    for object in &mut scene.objects {
        for mesh in &mut object.meshes {
            mesh.materials = Materials::new(override_material(side));
        }
    }
    scene
}
fn draws(scene: &Scene3d, objects: impl Iterator<Item = usize>) -> Result<Vec<DrawDesc>, String> {
    let all: Vec<_> = (0..scene.objects.len()).collect();
    let mut out = Vec::new();
    for object in objects {
        if let PassDesc::Color { draws, .. } = describe::color(&Layer3d {
            scene,
            object,
            depth_set: &all,
            background: false,
        })? {
            out.extend(draws);
        }
    }
    Ok(out)
}
fn adjust(draw: &mut DrawDesc, projection: &Mat4, fragment: &str) {
    draw.params.fragment_shader = fragment.into();
    draw.params.shader_name = format!(
        "viewport chrome {}",
        if fragment == DEPTH_FRAGMENT {
            "depth"
        } else {
            "IDs"
        }
    );
    draw.params.tone_mapping = crate::three::program::ToneMapping::None;
    draw.params.output_srgb = false;
    if let Some((_, p)) = draw
        .uniforms
        .iter_mut()
        .find(|(n, _)| n == "projectionMatrix")
    {
        *p = projection.to_vec();
    }
    draw.state.blend = false;
    draw.state.depth_write = true;
    draw.state.depth_test = true;
    draw.state.color_write = true;
    draw.state.color_mask = None;
}
impl Scene3dRenderer {
    /// Depth plane: exact f32 window-depth bits split 11/11/10 into binary
    /// fractions exactly representable in half float. Coverage plane: 22-bit visible
    /// object ID in RG; B=0/.5/1 for none/selected/active coverage, A=1.
    #[allow(clippy::too_many_arguments)]
    pub fn render_chrome(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &Scene3d,
        size: (u32, u32),
        rect: [f64; 4],
        selected: &[String],
        active: &[String],
        depth_packet: &wgpu::TextureView,
        coverage: &wgpu::TextureView,
    ) -> Result<(), Error3d> {
        let (w, h) = size;
        if !self
            .chrome
            .as_ref()
            .is_some_and(|t| t.depth.width() == w && t.depth.height() == h)
        {
            self.chrome = Some(Targets::new(device, w, h));
        }
        self.start_generation();
        let mut transform = crate::three::math::IDENTITY;
        transform[0] = rect[2] / f64::from(w);
        transform[5] = rect[3] / f64::from(h);
        transform[12] = (2.0 * rect[0] + rect[2] - f64::from(w)) / f64::from(w);
        transform[13] = (f64::from(h) - 2.0 * rect[1] - rect[3]) / f64::from(h);
        let projection = crate::three::math::multiply(
            &transform,
            &scene.camera.projection_matrix(scene.width, scene.height),
        );
        let depth_scene = scene_override(scene, Side::Front);
        let mut desc = draws(&depth_scene, 0..scene.objects.len())?;
        let mut depth_draws = Vec::new();
        let mut id_draws = Vec::new();
        for d in &mut desc {
            adjust(d, &projection, DEPTH_FRAGMENT);
            if let Some(prepared) = self.prepare(device, queue, encoder, &depth_scene, d, 1)? {
                depth_draws.push(prepared);
            }
            adjust(d, &projection, ID_FRAGMENT);
            let id = d.object as u32 + 1;
            if let Some((_, value)) = d.uniforms.iter_mut().find(|(n, _)| n == "diffuse") {
                *value = vec![
                    f64::from(id & 2047) / 2048.0,
                    f64::from(id >> 11) / 2048.0,
                    0.0,
                ];
            }
            if let Some(prepared) = self.prepare(device, queue, encoder, &depth_scene, d, 1)? {
                id_draws.push(prepared);
            }
        }
        let mask_scene = scene_override(scene, Side::Double);
        let mut mask_draws = Vec::new();
        // Today's mask draws selected red, then active green, with no depth.
        for (ids, kind) in [(selected, 0.5), (active, 1.0)] {
            let objects = (0..scene.objects.len()).filter(|i| ids.contains(&scene.objects[*i].id));
            for mut d in draws(&mask_scene, objects)? {
                if d.state.topology != describe::Topology::Triangles {
                    continue;
                }
                adjust(&mut d, &projection, ID_FRAGMENT);
                d.state.depth_write = false;
                d.state.depth_test = false;
                d.state.color_mask = Some(wgpu::ColorWrites::BLUE.bits());
                if let Some((_, value)) = d.uniforms.iter_mut().find(|(n, _)| n == "diffuse") {
                    *value = vec![0.0, 0.0, kind];
                }
                if let Some(prepared) = self.prepare(device, queue, encoder, &mask_scene, &d, 1)? {
                    mask_draws.push(prepared);
                }
            }
        }
        let targets = self.chrome.as_ref().expect("created above");
        for (target, clear, list) in [
            (depth_packet, packet(1.0), &depth_draws),
            (
                coverage,
                wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
                &id_draws,
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native viewport depth and IDs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for draw in list {
                draw.draw(&mut pass);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("unoccluded viewport selection"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: coverage,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for draw in &mask_draws {
                draw.draw(&mut pass);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn depth_packet_round_trips_every_exponent_and_mantissa_boundary() {
        for exponent in 0..255u32 {
            for mantissa in [0, 1, 2047, 2048, 0x3fffff, 0x7fffff] {
                let bits = (exponent << 23) | mantissa;
                let channels = [bits & 2047, (bits >> 11) & 2047, bits >> 22];
                let transported = channels.map(|c| {
                    pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(c as f32 / 2048.0))
                });
                let restored = transported.map(|v| (v * 2048.0).round() as u32);
                assert_eq!(restored[0] | restored[1] << 11 | restored[2] << 22, bits);
            }
        }
    }
}
