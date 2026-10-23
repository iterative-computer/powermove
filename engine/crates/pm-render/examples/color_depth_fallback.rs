//! Replay the actual WebGL runner's answers for the synthetic shader/effect
//! project captured by corpus/src/color-depth-fallback.ts. Build before taking
//! the Electron lock; that harness calls this binary between runner rounds.
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("external capture directory"),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    assert!(!dir.canonicalize().unwrap().starts_with(root));
    std::fs::create_dir_all(dir.join("requests")).unwrap();
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join("project.json")).unwrap()).unwrap();
    let document =
        pm_model::Document::new(Some(pm_model::json::parse(&value.to_string()).unwrap()), 1);
    assert_eq!(value["colorDepth"], 32);
    let mut renderer = Renderer::headless().unwrap();
    let mut definitions = pm_render::all_builtin_definitions();
    definitions.definitions.push(
        serde_json::from_slice(&std::fs::read(dir.join("definition.json")).unwrap()).unwrap(),
    );
    assert!(renderer.set_definitions(&definitions).unwrap().is_empty());
    renderer.set_shader_fallback_runner(true);
    // A first-use user program goes through the real ANGLE transport even
    // when native translation succeeds. This exercises both production
    // shader/effect runner paths without constructing a program WebGL refuses
    // (for example incompatible sampler types assigned to the same unit).
    renderer
        .effects_mut()
        .parity
        .configure(Some("color-depth-fallback-proof".into()), None);
    let request = RenderRequest::sized(Size::new(64, 32), Precision::Float32);
    let mut answered = 0;
    loop {
        match renderer.render(
            &document,
            value["compId"].as_str().unwrap(),
            pm_protocol::common::Ticks::new(0).unwrap(),
            &request,
            &MapResolver::default(),
        ) {
            Err(RenderError::ShaderFallbackPending(requests)) => {
                let mut missing = false;
                for request in requests {
                    assert_eq!(
                        request.color_depth, 32,
                        "float32 project must ask for float32 runner"
                    );
                    let key = &request.key;
                    let textures = renderer.fallback_textures(key).unwrap_or_default();
                    for (index, texture) in textures.iter().enumerate() {
                        assert!(texture.rgba16f.is_empty(), "no half storage at depth32");
                        let bits = texture.rgba32f.as_ref().expect("float32 texture");
                        assert!(
                            bits.iter().any(|bits| {
                                let value = f32::from_bits(*bits);
                                pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(value))
                                    .to_bits()
                                    != *bits
                            }),
                            "the effect witness must carry values binary16 cannot represent"
                        );
                        std::fs::write(
                            dir.join("requests").join(format!("{key}-{index}.f32")),
                            pm_render::shader_layers::fallback::float_bytes(bits),
                        )
                        .unwrap();
                    }
                    std::fs::write(
                        dir.join("requests").join(format!("{key}.json")),
                        serde_json::to_vec_pretty(&request).unwrap(),
                    )
                    .unwrap();
                    let answer = dir.join("pixels").join(format!("{key}.f32"));
                    if answer.exists() {
                        let bytes = std::fs::read(answer).unwrap();
                        assert_eq!(
                            bytes.len(),
                            request.width as usize * request.height as usize * 16
                        );
                        let bits = pm_render::shader_layers::fallback::float_bits(&bytes);
                        assert!(
                            bits.iter().any(|bits| {
                                let value = f32::from_bits(*bits);
                                pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(value))
                                    .to_bits()
                                    != *bits
                            }),
                            "runner output must distinguish float32 from a half-float transport"
                        );
                        renderer.put_fallback_pixels(
                            key.clone(),
                            pm_render::shader_layers::FallbackPixels {
                                width: request.width,
                                height: request.height,
                                rgba16f: Vec::new(),
                                rgba32f: Some(bits),
                            },
                        );
                        answered += 1;
                    } else {
                        missing = true;
                    }
                }
                if missing {
                    std::process::exit(2);
                }
                assert!(answered <= 8, "runner failed to settle");
            }
            Ok(frame) => {
                assert!(frame.failures.is_empty(), "{:?}", frame.failures);
                let pixels = renderer.read_rgba(&frame).unwrap();
                let bytes = std::fs::read(dir.join("reference.f32")).unwrap();
                let reference: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                assert_eq!(pixels.len(), reference.len());
                let byte = |f: f32| (f.clamp(0., 1.) * 255.).round() as u8;
                let different_channels = pixels
                    .iter()
                    .zip(&reference)
                    .filter(|(a, b)| byte(**a) != byte(**b))
                    .count();
                let max_cv = pixels
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| f64::from((a - b).abs()) * 255.)
                    .fold(0., f64::max);
                let different_bits = pixels
                    .iter()
                    .zip(&reference)
                    .filter(|(a, b)| a.to_bits() != b.to_bits())
                    .count();
                std::fs::write(
                    dir.join("native.f32"),
                    pixels
                        .iter()
                        .flat_map(|p| p.to_le_bytes())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let half_control_differences = reference
                    .iter()
                    .filter(|value| {
                        pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(**value))
                            .to_bits()
                            != value.to_bits()
                    })
                    .count();
                std::fs::write(dir.join("parity.json"), serde_json::to_vec_pretty(&json!({"depth":32,"answeredRequests":answered,"differentChannels":different_channels,"differentFloatBits":different_bits,"halfControlDifferences":half_control_differences,"maxCv":max_cv,"pass":different_bits == 0})).unwrap()).unwrap();
                assert!(answered >= 2, "must exercise shader and effect fallback");
                assert_eq!(
                    different_channels, 0,
                    "shader/effect output differs from today's app"
                );
                assert_eq!(
                    different_bits, 0,
                    "float32 shader/effect proof requires exact working values"
                );
                assert!(
                    half_control_differences > 0,
                    "the reference must detect half-float transport rounding"
                );
                println!(
                    "float32 shader/effect: {answered} requests, {different_channels} byte differences, {different_bits} float-bit differences, max {max_cv} CV"
                );
                return;
            }
            Err(error) => panic!("{error}"),
        }
    }
}
