use pm_studio::{environment::EnvironmentMap, terrain::Terrain};
#[test]
fn hdr_and_exr_keep_linear_high_dynamic_range() {
    let image =
        image::Rgb32FImage::from_raw(2, 2, vec![16., 0.5, 2., 1., 2., 3., 0., 0., 0., 7., 8., 9.])
            .unwrap();
    for (ext, format) in [
        ("hdr", image::ImageFormat::Hdr),
        ("exr", image::ImageFormat::OpenExr),
    ] {
        let path =
            std::env::temp_dir().join(format!("pm-studio-hdr-test-{}.{ext}", std::process::id()));
        image.save_with_format(&path, format).unwrap();
        let map = EnvironmentMap::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!((map.width, map.height), (2, 2));
        assert!((map.pixels[0].x - 16.).abs() < 0.1);
        assert!((map.pixels[0].y - 0.5).abs() < 0.02);
    }
}
#[test]
fn grayscale_images_become_normalized_heightfields() {
    let image = image::GrayImage::from_pixel(4, 4, image::Luma([128]));
    let path =
        std::env::temp_dir().join(format!("pm-studio-height-test-{}.png", std::process::id()));
    image.save(&path).unwrap();
    let mut terrain = Terrain {
        resolution: 8,
        ..Default::default()
    };
    terrain.load_height_image(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    terrain.validate(1).unwrap();
    assert_eq!(terrain.heights.len(), 81);
    assert!((terrain.sample(0., 0.).0 - 10. * 128. / 255.).abs() < 0.01);
}
