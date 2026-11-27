use pm_studio3d_engine::{gpu::Renderer, math::v3, scene::Material, swatch};
#[test]
fn swatches_render_material_content_and_refuse_unresolved_screens() {
    let mut renderer = Renderer::new(swatch::SIZE, swatch::SIZE).expect("Metal or Vulkan required");
    let red = Material::dielectric(v3(0.8, 0.05, 0.02), 0.4);
    let blue = Material::dielectric(v3(0.02, 0.05, 0.8), 0.4);
    let a = swatch::render(&mut renderer, red).unwrap();
    let b = swatch::render(&mut renderer, blue).unwrap();
    assert_ne!(a, b);
    assert_eq!(a.len(), 96 * 96 * 4);
    assert!(a.chunks_exact(4).all(|p| p[3] == 255));
    assert_eq!(a, swatch::render(&mut renderer, red).unwrap());
    assert!(swatch::render(&mut renderer, Material::display(1.)).is_err());
}
