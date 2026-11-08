//! Portable validation of the production source shader, including log transfers.
#[test]
fn source_shader_validates_for_every_working_storage_format() {
    for format in ["rgba8unorm", "rgba16float", "rgba32float"] {
        // Composed exactly as pm-render's `pipelines.rs` builds the source pass.
        let shader = format!(
            "{}\n{}\n{}\n{}",
            pm_color::wgsl::COLOR,
            pm_color::tonemap::wgsl(),
            pm_color::camera_log::wgsl(),
            include_str!("../../pm-render/src/shaders/source.wgsl").replace("OUT_FORMAT", format)
        );
        let module = naga::front::wgsl::parse_str(&shader)
            .unwrap_or_else(|e| panic!("{format}: {}", e.emit_to_string(&shader)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
