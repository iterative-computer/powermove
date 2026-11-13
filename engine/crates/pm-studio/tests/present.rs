//! Per-layer output into textures, as the native engine draws 3D layers.

use pm_studio::gpu::{PresentTarget, Renderer};
use pm_studio::scene::preset;

fn read(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (texture.width(), texture.height());
    let buffer = r.device().create_buffer(&wgpu::BufferDescriptor { label: None, size: u64::from(w * h * 4), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = r.device().create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) } },
        texture.size(),
    );
    r.queue().submit([enc.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = r.device().poll(wgpu::PollType::wait_indefinitely());
    let out = buffer.slice(..).get_mapped_range().expect("mapped").to_vec();
    out
}

#[test]
fn each_layer_receives_only_its_own_pixels() {
    let Ok(mut r) = Renderer::new(256, 144) else { return }; // no GPU: nothing to test
    // chrome: a mirror sphere (0), a glass sphere (1), a lacquered tablet (2).
    let s = preset("chrome", "space-gray", 0.0).unwrap();
    r.set_scene(&s, &s.prepare());
    r.set_void(None);
    r.trace(8, 6, true, 64);
    let mut coverage = Vec::new();
    for layer in [0u32, 1] {
        let texture = r.device().create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 256, height: 144, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        r.present_to(0, PresentTarget { view: &view, format: wgpu::TextureFormat::Rgba8Unorm, layer: Some(layer), include_void: false, flip_y: true });
        let px = read(&r, &texture);
        coverage.push(px.chunks(4).map(|p| p[3] > 0).collect::<Vec<_>>());
    }
    let count = |c: &Vec<bool>| c.iter().filter(|&&x| x).count();
    assert!(count(&coverage[0]) > 500, "mirror sphere: {}", count(&coverage[0]));
    assert!(count(&coverage[1]) > 300, "glass sphere: {}", count(&coverage[1]));
    // No pixel belongs to both layers, and the void stays transparent.
    assert_eq!(coverage[0].iter().zip(&coverage[1]).filter(|(a, b)| **a && **b).count(), 0);
    assert!(count(&coverage[0]) + count(&coverage[1]) < 256 * 144 / 2);
}
