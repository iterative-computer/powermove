//! Desktop Canvas2D uses BGRA N32. Readback exposes RGBA in either format.
#[test]
fn n32_readback_has_rgba_channel_order() {
    let gpu = pm_gpu::Gpu::new("N32 readback").expect("GPU");
    let rgba = [11u8, 79, 237, 161, 219, 53, 7, 103];
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ] {
        let mut bytes = rgba;
        if format == wgpu::TextureFormat::Bgra8Unorm {
            bytes.swap(0, 2);
            bytes.swap(4, 6);
        }
        let target = pm_gpu::Target::new(
            &gpu.device,
            "N32 channel test",
            2,
            1,
            format,
            wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        );
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let actual = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &target.texture).unwrap();
        assert_eq!(actual, rgba.map(|v| f32::from(v) / 255.));
    }
}
