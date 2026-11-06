//! E08 independent per-pixel equations from layer-effects.ts, compound-blur.ts,
//! set-channels.ts and layer-compositing.ts at c7f46a056. Shaders are inputs to
//! these tests, never their oracle.
use super::color_contract::{from_hls, hls, quantize};
use super::*;

type Pixel = [f64; 4];

fn solid(pixel: Pixel) -> String {
    format!(
        "void main(){{fragColor=vec4({},{},{},{});}}",
        pixel[0], pixel[1], pixel[2], pixel[3]
    )
}
fn scene(id: &str, version: u32, params: Value, maps: &[(&str, &str)], owner: &str) -> Value {
    let inputs = maps
        .iter()
        .map(|(key, layer)| (key.to_string(), json!({"layerId":layer,"stage":"source"})))
        .collect::<serde_json::Map<_, _>>();
    fixture(id, version, params, Value::Object(inputs), Some(owner))
}
fn hidden(value: &mut Value, id: &str, code: &str) {
    let mut source = layer(
        id,
        "shader",
        json!({"w":32,"h":32,"code":code,"uniforms":{}}),
        0.,
        0.,
        1.,
    );
    source["on"] = json!(false);
    value["layers"].as_array_mut().unwrap().push(source);
}
fn at(pixels: &[f32], x: usize, y: usize) -> Pixel {
    std::array::from_fn(|i| pixels[(y * 32 + x) * 4 + i] as f64)
}
fn close(got: Pixel, want: Pixel, context: &str) {
    for i in 0..4 {
        assert!(
            (got[i] - want[i]).abs() <= 1. / 1024.,
            "{context}, channel {i}: {got:?} != {want:?}"
        );
    }
}
fn uniform(pixels: &[f32], want: Pixel, context: &str) {
    for p in pixels.chunks_exact(4) {
        close(std::array::from_fn(|i| p[i] as f64), want, context);
    }
}
fn straight(c: Pixel) -> [f64; 3] {
    if c[3] > 1e-7 {
        [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
    } else {
        [0.; 3]
    }
}
fn luma(c: [f64; 3]) -> f64 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

/// `set-channels.ts`: Alpha, Red, Green, Blue, Luminance, Full On, Full Off.
fn channel_choice(sample: Pixel, choice: usize) -> f64 {
    let c = straight(sample);
    [sample[3], c[0], c[1], c[2], luma(c), 1., 0.][choice]
}

#[test]
fn e08_set_channels_takes_each_output_from_its_layer_and_channel() {
    let mut renderer = Renderer::headless().unwrap();
    let owner = [0.2, 0.3, 0.1, 0.5];
    let a = [0.32, 0.08, 0.4, 0.8];
    let b = [0.7, 0.2, 0.1, 1.];
    for depth in [16, 32] {
        for offset in 0..7 {
            let choices = [offset, (offset + 2) % 7, (offset + 4) % 7, (offset + 5) % 7];
            let mut value = scene(
                "setChannels",
                1,
                json!({"redChannel":choices[0],"greenChannel":choices[1],
                    "blueChannel":choices[2],"alphaChannel":choices[3]}),
                &[("redLayer", "a"), ("blueLayer", "b"), ("alphaLayer", "a")],
                &solid(owner),
            );
            hidden(&mut value, "a", &solid(a));
            hidden(&mut value, "b", &solid(b));
            let sources = [
                quantize(a, depth),
                quantize(owner, depth),
                quantize(b, depth),
                quantize(a, depth),
            ];
            let result: [f64; 4] = std::array::from_fn(|i| channel_choice(sources[i], choices[i]));
            let alpha = result[3].clamp(0., 1.);
            let expected = [
                result[0] * alpha,
                result[1] * alpha,
                result[2] * alpha,
                alpha,
            ];
            uniform(
                &draw(&mut renderer, &value, &MapResolver::default(), depth),
                quantize(expected, depth),
                &format!("depth {depth}, channels {choices:?}"),
            );
        }
    }
}

#[test]
fn e08_set_channels_defaults_keep_the_owner_and_clip_outside_it() {
    let mut renderer = Renderer::headless().unwrap();
    let owner = [0.2, 0.3, 0.1, 0.5];
    let value = scene("setChannels", 1, json!({}), &[], &solid(owner));
    uniform(
        &draw(&mut renderer, &value, &MapResolver::default(), 32),
        quantize(owner, 32),
        "identity",
    );
}

/// `compositing` of Blend: `straight`, HSL and premultiplied crossfade.
fn blend_expected(original: Pixel, secondary: Pixel, mode: u32, percent: f64) -> Pixel {
    let amount = 1. - (percent / 100.).clamp(0., 1.);
    if mode == 4 {
        return std::array::from_fn(|i| original[i] + (secondary[i] - original[i]) * amount);
    }
    let a = straight(original);
    let b = straight(secondary);
    let color: [f64; 3] = match mode {
        2 => std::array::from_fn(|i| a[i].min(b[i])),
        3 => std::array::from_fn(|i| a[i].max(b[i])),
        _ => {
            let peak = a.into_iter().fold(1., f64::max);
            let ah = hls(a.map(|v| (v / peak).clamp(0., 1.)));
            let bh = hls(b.map(|v| v.clamp(0., 1.)));
            let saturation = if mode == 1 { ah[2] * bh[2] } else { bh[2] };
            from_hls([bh[0], ah[1], saturation]).map(|v| v * peak)
        }
    };
    let t = amount * secondary[3];
    let mix: [f64; 3] = std::array::from_fn(|i| (a[i] + (color[i] - a[i]) * t) * original[3]);
    [mix[0], mix[1], mix[2], original[3]]
}

#[test]
fn e08_blend_modes_amount_alpha_and_hdr_match_the_reference_equations() {
    let mut renderer = Renderer::headless().unwrap();
    let cases: [(Pixel, Pixel); 3] = [
        ([0.4, 0.2, 0.1, 1.], [0.1, 0.6, 0.3, 1.]),
        ([0.2, 0.1, 0.05, 0.5], [0.2, 0.3, 0.05, 0.5]),
        ([1.75, 0.5, 0.25, 1.], [0.25, 0.25, 0.75, 0.75]),
    ];
    for depth in [16, 32] {
        for (original, secondary) in cases {
            for mode in 0..5u32 {
                for percent in [0., 35., 100.] {
                    let mut value = scene(
                        "blend",
                        1,
                        json!({"mode":mode,"blendWithOriginal":percent}),
                        &[("map", "second")],
                        &solid(original),
                    );
                    hidden(&mut value, "second", &solid(secondary));
                    let expected = blend_expected(
                        quantize(original, depth),
                        quantize(secondary, depth),
                        mode,
                        percent,
                    );
                    uniform(
                        &draw(&mut renderer, &value, &MapResolver::default(), depth),
                        quantize(expected, depth),
                        &format!("depth {depth}, mode {mode}, {percent}%, {original:?}"),
                    );
                }
            }
        }
    }
}

#[test]
fn e08_blend_center_uses_the_map_at_its_own_size_and_stretch_fills() {
    let mut renderer = Renderer::headless().unwrap();
    let owner = [0.5, 0.25, 0.75, 1.];
    let second = [1., 0., 0., 1.];
    for (placement, inside_only) in [(0, true), (1, false)] {
        let mut value = scene(
            "blend",
            1,
            json!({"mode":4,"placement":placement}),
            &[("map", "small")],
            &solid(owner),
        );
        let mut small = layer(
            "small",
            "solid",
            json!({"w":16,"h":16,"color":"#FF0000"}),
            8.,
            8.,
            1.,
        );
        small["on"] = json!(false);
        value["layers"].as_array_mut().unwrap().push(small);
        let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
        close(at(&pixels, 16, 16), second, "the map's middle");
        let edge = if inside_only { [0.; 4] } else { second };
        close(at(&pixels, 2, 2), edge, "outside the centred map");
    }
}

/// `extractChannel`.
fn extract(c: Pixel, channel: usize, invert: bool) -> Pixel {
    let mut rgb = straight(c);
    let mut alpha = c[3];
    if invert {
        rgb = rgb.map(|v| 1. - v);
        alpha = 1. - alpha;
    }
    let rgb = match channel {
        1 => [luma(rgb); 3],
        2..=4 => [rgb[channel - 2]; 3],
        5 => [alpha; 3],
        _ => rgb,
    };
    [rgb[0], rgb[1], rgb[2], alpha.clamp(0., 1.)]
}
/// `blendColor`, base then source.
fn blend_color(b: f64, s: f64, mode: u32) -> f64 {
    match mode {
        1 => b * s,
        2 => 1. - (1. - b) * (1. - s),
        3 => {
            if b < 0.5 {
                2. * b * s
            } else {
                1. - 2. * (1. - b) * (1. - s)
            }
        }
        4 => b.min(s),
        5 => b.max(s),
        6 => (b - s).abs(),
        7 => b + s,
        8 => b - s,
        _ => s,
    }
}
struct Calculation {
    input: usize,
    invert_input: bool,
    second: usize,
    invert_second: bool,
    opacity: f64,
    mode: u32,
    preserve: bool,
}
fn calculate(original: Pixel, second: Pixel, c: &Calculation) -> Pixel {
    let base = extract(original, c.input, c.invert_input);
    let mut source = extract(second, c.second, c.invert_second);
    source[3] *= (c.opacity / 100.).clamp(0., 1.);
    let (ab, r#as) = (base[3], source[3]);
    let rgb: [f64; 3] = std::array::from_fn(|i| {
        (1. - r#as) * base[i] * ab
            + (1. - ab) * source[i] * r#as
            + blend_color(base[i], source[i], c.mode) * ab * r#as
    });
    let result = [rgb[0], rgb[1], rgb[2], ab + r#as * (1. - ab)];
    if !c.preserve {
        return result;
    }
    let s = straight(result);
    [
        s[0] * original[3],
        s[1] * original[3],
        s[2] * original[3],
        original[3],
    ]
}

#[test]
fn e08_calculations_channels_inversions_modes_and_transparency() {
    let mut renderer = Renderer::headless().unwrap();
    let original = [0.3, 0.15, 0.05, 0.6];
    let second = [0.1, 0.5, 0.2, 0.75];
    let depth = 32;
    let (o, s) = (quantize(original, depth), quantize(second, depth));
    for input in 0..6 {
        for mode in 0..9 {
            for (invert_input, invert_second, preserve, opacity) in [
                (false, false, true, 100.),
                (true, false, false, 60.),
                (false, true, false, 100.),
                (true, true, true, 0.),
            ] {
                let c = Calculation {
                    input,
                    invert_input,
                    second: (input + 2) % 6,
                    invert_second,
                    opacity,
                    mode,
                    preserve,
                };
                let mut value = scene(
                    "calculations",
                    1,
                    json!({"inputChannel":c.input,"invertInput":c.invert_input,
                        "secondChannel":c.second,"secondOpacity":c.opacity,
                        "invertSecond":c.invert_second,"mode":c.mode,
                        "preserveTransparency":c.preserve}),
                    &[("map", "second")],
                    &solid(original),
                );
                hidden(&mut value, "second", &solid(second));
                uniform(
                    &draw(&mut renderer, &value, &MapResolver::default(), depth),
                    quantize(calculate(o, s, &c), depth),
                    &format!(
                        "input {input}, mode {mode}, invert {invert_input}/{invert_second}, preserve {preserve}"
                    ),
                );
            }
        }
    }
}

#[test]
fn e08_calculations_without_a_second_layer_is_identity_until_it_derives_a_channel() {
    let mut renderer = Renderer::headless().unwrap();
    let original = [0.3, 0.15, 0.05, 0.6];
    let depth = 32;
    let value = scene("calculations", 1, json!({}), &[], &solid(original));
    uniform(
        &draw(&mut renderer, &value, &MapResolver::default(), depth),
        quantize(original, depth),
        "identity",
    );
    let c = Calculation {
        input: 2,
        invert_input: false,
        second: 0,
        invert_second: false,
        opacity: 100.,
        mode: 0,
        preserve: true,
    };
    let value = scene(
        "calculations",
        1,
        json!({"inputChannel":2}),
        &[],
        &solid(original),
    );
    let o = quantize(original, depth);
    uniform(
        &draw(&mut renderer, &value, &MapResolver::default(), depth),
        quantize(calculate(o, [0.; 4], &c), depth),
        "red into gray",
    );
}

/// The owner: red rises by 1/32 per pixel, green falls with y from the top.
const RAMP: &str = "void main(){fragColor=vec4(gl_FragCoord.x/32.,gl_FragCoord.y/32.,0.,1.);}";
fn ramp(x: f64, y: f64) -> Pixel {
    [x / 32., 1. - y / 32., 0., 1.]
}
/// Revision 2's piecewise map: 128/255 is neutral on byte maps, else 0.5.
fn displacement(value: f64, midpoint: f64, version: u32) -> f64 {
    let value = value.clamp(0., 1.);
    if version == 1 {
        return value * 2. - 1.;
    }
    (value - midpoint)
        / if value >= midpoint {
            1. - midpoint
        } else {
            midpoint
        }
}

#[test]
fn e08_displacement_channels_shift_the_owner_by_the_mapped_amount() {
    let mut renderer = Renderer::headless().unwrap();
    // Straight map value 0.8, 0.4, 0.2 at alpha 0.5.
    let map = [0.4, 0.2, 0.1, 0.5];
    let (r, g, b) = (0.8, 0.4, 0.2);
    let h = hls([r, g, b]);
    let channels = [r, g, b, 0.5, luma([r, g, b]), h[0], h[1], h[2]];
    for depth in [16, 32] {
        for version in [1, 2] {
            for (channel, source) in channels.iter().enumerate() {
                let mut value = scene(
                    "displacementMap",
                    version,
                    json!({"horizontal":channel,"maxHorizontal":6,"vertical":8,"maxVertical":4,"behavior":1}),
                    &[("map", "field")],
                    RAMP,
                );
                hidden(&mut value, "field", &solid(map));
                let pixels = draw(&mut renderer, &value, &MapResolver::default(), depth);
                let dx = displacement(*source, 0.5, version) * 6.;
                // Version 1 maps Off to 0.5 as well, which is its neutral.
                let (x, y) = (16.5 + dx, 16.5);
                close(
                    at(&pixels, 16, 16),
                    ramp(x, y),
                    &format!("depth {depth}, v{version}, channel {channel}"),
                );
            }
        }
    }
}

#[test]
fn e08_byte_projects_use_the_representable_neutral() {
    let mut renderer = Renderer::headless().unwrap();
    // 128 of 255 must not move anything in an 8-bit project (revision 2).
    let neutral = 128. / 255.;
    let map = [neutral, neutral, 0., 1.];
    let mut value = scene(
        "displacementMap",
        2,
        json!({"horizontal":0,"maxHorizontal":200,"vertical":1,"maxVertical":200,"behavior":1}),
        &[("map", "field")],
        RAMP,
    );
    hidden(&mut value, "field", &solid(map));
    close(
        at(
            &draw(&mut renderer, &value, &MapResolver::default(), 8),
            16,
            16,
        ),
        ramp(16.5, 16.5),
        "byte neutral",
    );
    // A float project keeps exact 0.5, so the same value moves the picture.
    let moved = displacement(neutral, 0.5, 2) * 200.;
    assert!(moved > 0.7);
    close(
        at(
            &draw(&mut renderer, &value, &MapResolver::default(), 16),
            8,
            16,
        ),
        ramp(8.5 + moved, 16.5 + moved),
        "float midpoint",
    );
}

#[test]
fn e08_displacement_edges_wrap_expand_and_placement() {
    let mut renderer = Renderer::headless().unwrap();
    let full_right = solid([1., 0.5, 0., 1.]);
    let params = |wrap: bool| json!({"horizontal":0,"maxHorizontal":8,"vertical":8,"maxVertical":0,"behavior":1,"wrap":wrap});
    let mut value = scene(
        "displacementMap",
        2,
        params(false),
        &[("map", "field")],
        RAMP,
    );
    hidden(&mut value, "field", &full_right);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 4, 8), ramp(12.5, 8.5), "inside");
    close(
        at(&pixels, 28, 8),
        [0.; 4],
        "displaced past the edge is empty",
    );
    let mut value = scene(
        "displacementMap",
        2,
        params(true),
        &[("map", "field")],
        RAMP,
    );
    hidden(&mut value, "field", &full_right);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 28, 8), ramp(4.5, 8.5), "wraps to the far side");
    // Center Map at half size: only the middle has a displacing map.
    let mut value = scene(
        "displacementMap",
        2,
        json!({"horizontal":0,"maxHorizontal":8,"vertical":8,"maxVertical":0,"behavior":0}),
        &[("map", "small")],
        RAMP,
    );
    let mut small = layer(
        "small",
        "solid",
        json!({"w":16,"h":16,"color":"#FF0000"}),
        8.,
        8.,
        1.,
    );
    small["on"] = json!(false);
    value["layers"].as_array_mut().unwrap().push(small);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 16, 8), ramp(24.5, 8.5), "over the map");
    // Empty map pixels read 0, a full negative displacement, as in the source.
    close(at(&pixels, 2, 8), [0.; 4], "beyond the map");
    let value = scene("displacementMap", 2, json!({}), &[], RAMP);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 10, 10), ramp(10.5, 10.5), "no map layer");
}

/// A bright vertical bar on transparent black, in owner pixels.
const BAR: &str = "void main(){float on=abs(gl_FragCoord.x-16.)<4.?1.:0.;fragColor=vec4(on);}";
fn bar(x: i32) -> f64 {
    if (x as f64 + 0.5 - 16.).abs() < 4. {
        1.
    } else {
        0.
    }
}
/// Direct Gaussian of the bar: the reference's taps at one-pixel stride.
fn compound(x: i32, sigma: f64) -> f64 {
    let taps = ((3. * sigma).ceil() as i32).clamp(1, 8);
    let (mut sum, mut total) = (0., 0.);
    for dy in -taps..=taps {
        for dx in -taps..=taps {
            let weight = (-0.5 * f64::from(dx * dx + dy * dy) / (sigma * sigma)).exp();
            if (0..32).contains(&(x + dx)) {
                sum += bar(x + dx) * weight;
            }
            total += weight;
        }
    }
    sum / total
}

#[test]
fn e08_compound_blur_follows_map_luminance_and_inversion() {
    let mut renderer = Renderer::headless().unwrap();
    for depth in [16, 32] {
        // maximumBlur 4 is sigma 2 at full map: three sigma stays below eight
        // taps, so the source reads level zero.
        for (shade, invert, sigma) in [(1., false, 2.), (0.5, false, 1.), (1., true, 0.)] {
            let map = [shade, shade, shade, 1.];
            let mut value = scene(
                "compoundBlur",
                1,
                json!({"maximumBlur":4,"invert":invert}),
                &[("map", "field")],
                BAR,
            );
            hidden(&mut value, "field", &solid(map));
            let pixels = draw(&mut renderer, &value, &MapResolver::default(), depth);
            let amount = luma([shade; 3]);
            let sigma = if invert {
                (1. - amount) * 2.
            } else {
                sigma * amount / shade
            };
            for x in [8, 10, 12, 14, 16] {
                let want = if sigma < 0.01 {
                    bar(x)
                } else {
                    compound(x, sigma)
                };
                close(
                    at(&pixels, x as usize, 16),
                    quantize([want; 4], depth),
                    &format!("depth {depth}, x {x}, shade {shade}, invert {invert}"),
                );
            }
        }
    }
}

#[test]
fn e08_compound_blur_without_a_map_or_radius_leaves_the_owner() {
    let mut renderer = Renderer::headless().unwrap();
    let value = scene("compoundBlur", 1, json!({"maximumBlur":4}), &[], BAR);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 14, 16), [1.; 4], "no map, bar");
    close(at(&pixels, 2, 16), [0.; 4], "no map, ground");
    let mut value = scene(
        "compoundBlur",
        1,
        json!({"maximumBlur":0}),
        &[("map", "field")],
        BAR,
    );
    hidden(&mut value, "field", &solid([1., 1., 1., 1.]));
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 14, 16), [1.; 4], "zero radius");
}

#[test]
fn e08_compound_blur_map_reads_luminance_in_place_when_stretched() {
    let mut renderer = Renderer::headless().unwrap();
    // The left half of the map is black: blur only begins at the middle.
    let half = "void main(){float on=gl_FragCoord.x<16.?0.:1.;fragColor=vec4(on,on,on,1.);}";
    let flat = "void main(){float on=abs(gl_FragCoord.y-16.)<4.?1.:0.;fragColor=vec4(on);}";
    let mut value = scene(
        "compoundBlur",
        1,
        json!({"maximumBlur":4}),
        &[("map", "field")],
        flat,
    );
    hidden(&mut value, "field", half);
    let pixels = draw(&mut renderer, &value, &MapResolver::default(), 32);
    close(at(&pixels, 6, 12), [1.; 4], "black map keeps the edge");
    close(at(&pixels, 6, 11), [0.; 4], "black map keeps the ground");
    // The bar's first row, read across it: the same profile as `bar` at x 12.
    let want = compound(12, 2.);
    close(at(&pixels, 20, 12), [want; 4], "white map blurs the edge");
}
