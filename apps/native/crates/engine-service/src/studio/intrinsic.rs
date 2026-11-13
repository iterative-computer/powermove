//! Sparse evaluated channels overlay immutable source values on each rebuild.
use super::*;

pub(super) fn apply(
    object: &Value,
    index: Option<usize>,
    material: &mut Material,
    gpu: &mut GpuMaterial,
    emission_color: Vec3,
    emission_intensity: f32,
    basic: bool,
) {
    let indexed = index.map(|i| &object["sourceMaterialProperties"][i.to_string()]);
    let property = |key: &str| {
        indexed
            .and_then(|v| v.get(key))
            .or_else(|| object["materialProperties"].get(key))
            .map(val)
    };
    let number = |key: &str| property(key).and_then(Value::as_f64).map(|v| v as f32);
    if let Some(opacity) = number("opacity") {
        gpu.extra[2] = opacity / 100.;
    }
    // glTF Basic/unlit materials only expose opacity in the pinned runtime.
    if basic {
        return;
    }
    if let Some(roughness) = number("roughness") {
        material.roughness = roughness;
        gpu.spec[0] = roughness.max(0.002);
    }
    if let Some(metallic) = number("metallic") {
        material.metallic = metallic;
        gpu.base[3] = metallic;
    }
    let color = property("emissionColor")
        .and_then(Value::as_str)
        .map(srgb_hex);
    let intensity = number("emissionIntensity");
    if color.is_some() || intensity.is_some() {
        material.emission =
            color.unwrap_or(emission_color) * intensity.unwrap_or(emission_intensity);
        gpu.emit[..3].copy_from_slice(&[
            material.emission.x,
            material.emission.y,
            material.emission.z,
        ]);
    }
    if let Some(response) = number("ambientResponse") {
        gpu.policy[0] = response / 100.;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn zero_original_intensity_retains_color_for_independent_edits() {
        let original = Material::dielectric(Vec3::splat(1.), 0.4);
        for (properties, expected) in [
            (json!({"emissionIntensity":2}), [2., 0., 0.]),
            (json!({"emissionColor":"#00FF00"}), [0., 0., 0.]),
            (
                json!({"emissionColor":"#00FF00","emissionIntensity":2}),
                [0., 2., 0.],
            ),
        ] {
            let mut material = original;
            let mut gpu = material.gpu();
            apply(
                &json!({"materialProperties":properties}),
                None,
                &mut material,
                &mut gpu,
                v3(1., 0., 0.),
                0.,
                false,
            );
            assert_eq!(&gpu.emit[..3], &expected);
        }
    }
    #[test]
    fn basic_only_changes_opacity_and_indexed_values_win() {
        let mut material = Material::dielectric(Vec3::splat(1.), 0.4);
        let mut gpu = material.gpu();
        let before = gpu;
        apply(
            &json!({"materialProperties":{"opacity":60,"roughness":0.9,
            "metallic":1,"emissionColor":"#FF0000","emissionIntensity":10,
            "ambientResponse":0},"sourceMaterialProperties":{"1":{"opacity":25}}}),
            Some(1),
            &mut material,
            &mut gpu,
            Vec3::ZERO,
            0.,
            true,
        );
        assert_eq!(gpu.extra[2], 0.25);
        assert_eq!(gpu.base, before.base);
        assert_eq!(gpu.spec, before.spec);
        assert_eq!(gpu.emit, before.emit);
        assert_eq!(gpu.policy, before.policy);
    }
}
