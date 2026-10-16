// The 2D compositing passes (plan section 8; legacy `legacy/gl/shaders.ts`).
//
// Prepended with `pm_color::wgsl::COLOR`.
//
// One quad per draw, as today's `PM.VERT`: the unit quad (0,0) (1,0) (0,1)
// (1,1) through a 3×3 matrix to target pixels, projective (w = p.z), with
// the content UV window `uv.xy + a · uv.zw`.
//
// Targets keep GL's row order, as today's framebuffers do (row 0 is the
// bottom of the picture): clip y is negated against `PM.VERT`, so the
// rasterizer sees the same edges in the same orientation as ANGLE's Metal
// back end does, and a pixel center on the shared edge of two quads goes
// to the same quad as today. Sources sampled from targets flip v
// (`u_fromFbo`); images and video frames are stored top row first. The
// renderer flips the finished frame once into a top-down target.
//
// Blending is the fixed-function premultiplied source-over (ONE,
// ONE_MINUS_SRC_ALPHA) set on the pipeline, as today's `blendFunc`.

struct Quad {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // xy: target size; zw: unused.
    res: vec4<f32>,
    // Content UV window: offset (xy), scale (zw).
    uv: vec4<f32>,
    // Solid: premultiplied color. Texture: x = alpha, y = 1 when the source
    // is in GL row order, z = 1 when it holds premultiplied E values to be
    // read into linear light.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> q: Quad;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(q.m0.xyz, q.m1.xyz, q.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var o: VsOut;
    o.uv = q.uv.xy + a * q.uv.zw;
    o.pos = vec4<f32>((p.x / q.res.x) * 2.0 - p.z, (p.y / q.res.y) * 2.0 - p.z, 0.0, p.z);
    return o;
}

// `FRAG_DRAW`: no mipmaps, bilinear, hard edges outside the UV window;
// `color.y` = 1 for sources in GL row order (`u_fromFbo`).
@fragment
fn fs_draw(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (q.color.y > 0.5) {
        uv = vec2<f32>(uv.x, 1.0 - uv.y);
    }
    var c = textureSampleLevel(tex, smp, uv, 0.0);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        c = vec4<f32>(0.0);
    }
    if (q.color.z > 0.5) {
        c = pm_premultiplied_e_to_linear(c);
    }
    return c * q.color.x;
}

// `FRAG_SOLID`: the premultiplied color.
@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4<f32> {
    return q.color;
}

// `FRAG_BACKGROUND_FILL` at full-frame coordinates (a region of interest
// samples the gradient where the full frame has it).
struct Fill {
    stops: array<vec4<f32>, 8>,
    // x: count, y: 1 linear / 2 radial, z: angle in degrees, w: region height.
    params: vec4<f32>,
    // Region origin (xy) and full output size (zw) in pixels.
    view: vec4<f32>,
    // x: 1 when the working encoding is linear light (the gradient is
    // interpolated in E, as today, then decoded).
    flags: vec4<f32>,
};

@group(1) @binding(2) var<uniform> fill: Fill;

fn step_amount(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 <= e0) {
        return select(0.0, 1.0, x >= e0);
    }
    return smoothstep(e0, e1, x);
}

@fragment
fn fs_background(in: VsOut) -> @location(0) vec4<f32> {
    // GL's v_st: x to the right, y up from the bottom edge. Row 0 of the
    // target is the bottom of the region (GL row order).
    let x = in.pos.x + fill.view.x;
    let y_from_bottom = in.pos.y + (fill.view.w - fill.view.y - fill.params.w);
    let st = vec2<f32>(x / fill.view.z, y_from_bottom / fill.view.w);
    let p = st - vec2<f32>(0.5);
    let a = radians(fill.params.z);
    var t: f32;
    if (fill.params.y > 1.5) {
        t = clamp(length(p) * 1.4142, 0.0, 1.0);
    } else {
        t = clamp(dot(p, vec2<f32>(cos(a), sin(a))) + 0.5, 0.0, 1.0);
    }
    var color = fill.stops[0].rgb;
    let count = i32(fill.params.x);
    for (var i = 0; i < 7; i++) {
        if (i >= count - 1) {
            break;
        }
        let left = fill.stops[i];
        let right = fill.stops[i + 1];
        let amount = step_amount(left.a, right.a, t);
        color = mix(color, right.rgb, amount);
    }
    if (fill.flags.x > 0.5) {
        color = pm_e_decode3(color);
    }
    return vec4<f32>(color, 1.0);
}

// The finished frame from GL row order to top-down rows (an exact copy).
@fragment
fn fs_flip(in: VsOut) -> @location(0) vec4<f32> {
    let h = i32(textureDimensions(tex).y);
    let at = vec2<i32>(i32(in.pos.x), h - 1 - i32(in.pos.y));
    return textureLoad(tex, at, 0);
}
