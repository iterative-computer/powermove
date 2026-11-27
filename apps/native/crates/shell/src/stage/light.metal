// The first-run stage's Powermove mark as a lit object, over the dimmed
// desktop: one full-surface pass, transparent everywhere but the mark and
// its light. The mark is a signed distance field (mark units); its body is a
// warm gradient with a bright rim, tone-mapped by an exposure that starts
// white-hot; its bloom and the soft light it spills under it are added to
// what lies beneath (alpha 0), the body covers it (alpha its coverage).
// Output is premultiplied, display-encoded sRGB (BGRA8Unorm).
#include <metal_stdlib>
using namespace metal;

struct Light {
    float2 size;       // output, device pixels
    float2 centre;     // mark origin, device pixels
    float scale;       // device pixels per mark unit
    float exposure;    // body tone mapping: high is white-hot
    float bloom;       // bloom around the mark
    float pool;        // light the mark spills onto the dim under it
    float mark;        // the mark drawn (0 hides it, its light stays)
    float time;        // seconds, for breathing and the body's drift
    float density;     // device pixels per point
    float reach;       // the mark's resting height, device pixels: its light's size
    float soft;        // the outline's softness, device pixels
    float iris;        // the body's colour drift (0–1)
    float angle;       // clockwise mark rotation, radians (0 = upright playhead)
    float pad;
    float4 domain;     // the field's rectangle in mark units: min.xy, max.xy
};

struct Out { float4 position [[position]]; };

vertex Out light_vertex(uint i [[vertex_id]]) {
    float2 q = float2(float((i << 1) & 2), float(i & 2));
    Out o;
    o.position = float4(q * 2. - 1., 0., 1.);
    return o;
}

static float encode(float v) {
    v = clamp(v, 0., 1.);
    return v <= 0.0031308 ? v * 12.92 : 1.055 * pow(v, 1. / 2.4) - 0.055;
}
static float3 encode3(float3 c) {
    return float3(encode(c.r), encode(c.g), encode(c.b));
}

fragment float4 light_fragment(Out in [[stage_in]], constant Light &u [[buffer(0)]],
                               texture2d<float> field [[texture(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    float2 px = in.position.xy;
    float h = u.size.y;
    float2 rel = (px - u.centre) / u.scale;
    float ca = cos(u.angle), sa = sin(u.angle);
    float2 p = float2(ca * rel.x + sa * rel.y, -sa * rel.x + ca * rel.y);
    // Distance to the mark, in mark units; outside the field's rectangle,
    // add the way to it.
    float2 lo = u.domain.xy, hi = u.domain.zw;
    float2 inside = clamp(p, lo, hi);
    float d = field.sample(s, (inside - lo) / (hi - lo)).r + length(p - inside);
    // White-hot, the outline swells and rounds (the field dilated by most of
    // its softness): the light rises as a dome, never as a thin white spike,
    // and resolves to the mark as the exposure comes down.
    float dpx = d * u.scale - 0.8 * u.soft;

    // The light, added to the dim: nothing of it is a ground.
    float3 add = float3(0.);
    // Breathing: a slow swell of the light, never a pulse.
    float breathe = 1. + 0.07 * sin(u.time * 6.2831853 / 3.4);
    float markH = u.reach;

    // The light on the dim under the mark: a wide, low pool.
    float2 pq = (px - float2(u.centre.x, u.centre.y + 0.78 * markH)) / float2(1.9 * markH, 0.42 * markH);
    add += u.pool * breathe * 0.075 * exp(-dot(pq, pq)) * float3(1.0, 0.52, 0.24);
    // Its reach: a broad halo, faint.
    float2 hq = (px - u.centre) / (2.0 * markH);
    add += u.pool * breathe * 0.010 * exp(-dot(hq, hq)) * float3(1.0, 0.58, 0.32);

    // Bloom: a tight glow and a wide one, warmer and whiter as the mark
    // burns; it falls off with the distance from the outline.
    float outside = max(dpx, 0.);
    float near = exp(-outside / (0.010 * h));
    float hot = clamp((u.exposure - 1.6) / 6., 0., 1.);
    // Kept close and warm: the mark is the light.
    float far = exp(-outside / ((0.060 + 0.035 * hot) * h));
    float3 glow = mix(float3(1.0, 0.46, 0.17), float3(1.0, 0.68, 0.42), hot);
    add += u.bloom * breathe * (0.55 * near + 0.20 * far) * glow;

    // The body: vibrant Powermove orange gradient with a subtle warm ember rim
    // (never a harsh white outline), tone-mapped by the exposure.
    float3 lit = float3(0.);
    float cover = 0.;
    if (u.mark > 0. && dpx < max(2., u.soft)) {
        float depth = max(-d, 0.);
        float v = clamp((p.y + 55.) / 112., 0., 1.);
        float3 top = float3(1.00, 0.56, 0.20);
        float3 mid = float3(0.96, 0.36, 0.06);
        float3 base = float3(0.76, 0.20, 0.04);
        float3 body = v < 0.45 ? mix(top, mid, v / 0.45) : mix(mid, base, (v - 0.45) / 0.55);
        body *= mix(1.0, 0.78, smoothstep(0., 16., depth));
        float rim = exp(-depth / 2.2);
        body += rim * float3(1.0, 0.48, 0.14) * 0.32;
        float2 sp = (p - float2(-4., -24.)) / float2(18., 24.);
        body += 0.26 * exp(-dot(sp, sp)) * float3(1.0, 0.68, 0.36);
        // Living colour once landed: a slow warm drift inside the body.
        float drift = u.time * 0.45 + p.x * 0.016 - p.y * 0.011;
        float3 tint = mix(mix(float3(1.0, 0.56, 0.32), float3(1.0, 0.74, 0.34), 0.5 + 0.5 * sin(drift)),
                          float3(1.0, 0.58, 0.26), 0.5 + 0.5 * sin(drift * 0.73 + 2.1));
        body = mix(body, body * tint * 1.18, 0.35 * u.iris * (1. - rim));
        body *= 1.45 * (0.96 + 0.04 * breathe);
        // The body glows with its own bloom on it, as before over black.
        lit = 1. - exp(-body * u.exposure) + add;
        float edgeWidth = max(max(u.density, 1.), u.soft);
        cover = clamp(0.5 - dpx / edgeWidth, 0., 1.) * u.mark;
    }
    // Premultiplied: the body covers what is under it; the light adds.
    return float4(encode3(lit) * cover + encode3(add) * (1. - cover), cover);
}
