// The first-run stage's room and the Powermove mark as a lit object: one
// full-surface pass. The mark is a signed distance field (mark units); its
// body is a warm gradient with a bright rim, tone-mapped by an exposure that
// starts white-hot; its bloom and the light it spills on the stage are
// added around it. Output is display-encoded sRGB (BGRA8Unorm).
#include <metal_stdlib>
using namespace metal;

struct Light {
    float2 size;       // output, device pixels
    float2 centre;     // mark origin, device pixels
    float scale;       // device pixels per mark unit
    float exposure;    // body tone mapping: high is white-hot
    float bloom;       // bloom around the mark
    float edge;        // light at the bottom edge
    float pool;        // light the mark spills onto the stage under it
    float mark;        // the mark drawn (0 hides it, its light stays)
    float time;        // seconds, for grain and breathing
    float density;     // device pixels per point
    float reach;       // the mark's resting height, device pixels: its light's size
    float atmosphere;  // the room's own light from above (Dia's room light)
    packed_float3 ground; // a window's ground (linear); zero: the room
    packed_float3 pad;
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
static float hash(float2 p) {
    float3 q = fract(float3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fragment float4 light_fragment(Out in [[stage_in]], constant Light &u [[buffer(0)]],
                               texture2d<float> field [[texture(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    float2 px = in.position.xy;
    float h = u.size.y;
    float2 p = (px - u.centre) / u.scale;
    // Distance to the mark, in mark units; outside the field's rectangle,
    // add the way to it.
    float2 lo = u.domain.xy, hi = u.domain.zw;
    float2 inside = clamp(p, lo, hi);
    float d = field.sample(s, (inside - lo) / (hi - lo)).r + length(p - inside);
    float dpx = d * u.scale;

    // The room: a warm dark ground, lifted a little above the centre. A
    // window: its own ground, a touch lighter under its top edge.
    float3 c;
    float3 ground = float3(u.ground);
    bool window = any(ground > 0.);
    if (window) {
        float lift = exp(-px.y / (0.5 * h));
        c = ground * (1. + 0.10 * lift);
    } else {
        float2 q = (px - float2(u.size.x * 0.5, h * 0.42)) / h;
        c = mix(float3(0.0068, 0.0068, 0.0080), float3(0.0018, 0.0018, 0.0022),
                smoothstep(0.0, 0.95, length(q * float2(0.8, 1.0))));
        // The room's light from above: a broad soft wash, warm at its heart
        // and cooler at its edge, that goes down as the house lights do.
        float2 a = (px - float2(u.size.x * 0.5, -0.25 * h)) / float2(0.95 * u.size.x, 0.95 * h);
        float wash = exp(-dot(a, a) * 2.2);
        c += u.atmosphere * wash * mix(float3(0.030, 0.034, 0.048), float3(0.090, 0.062, 0.040), wash);
    }

    // Breathing: a slow swell of the light, never a pulse.
    float breathe = 1. + 0.07 * sin(u.time * 6.2831853 / 3.4);
    float markH = u.reach;

    // The light on the stage under the mark: a wide, low pool.
    float2 pq = (px - float2(u.centre.x, u.centre.y + 0.78 * markH)) / float2(1.9 * markH, 0.42 * markH);
    c += u.pool * breathe * 0.075 * exp(-dot(pq, pq)) * float3(1.0, 0.52, 0.24);
    // Its reach into the room: a broad halo, faint.
    float2 hq = (px - u.centre) / (2.0 * markH);
    c += u.pool * breathe * 0.020 * exp(-dot(hq, hq)) * float3(1.0, 0.58, 0.32);

    // The edge light: warm at the bottom, around where the mark comes up.
    float ex = (px.x - u.centre.x) / (0.46 * u.size.x);
    float ey = (h - px.y) / (0.11 * h);
    c += u.edge * exp(-ey - ex * ex) * float3(0.95, 0.30, 0.09);
    c += u.edge * 0.12 * exp(-ey * 0.45 - ex * ex * 0.6) * float3(0.55, 0.16, 0.06);

    // Bloom: a tight glow and a wide one, warmer and whiter as the mark
    // burns; it falls off with the distance from the outline.
    float outside = max(dpx, 0.);
    float near = exp(-outside / (0.010 * h));
    float far = exp(-outside / (0.075 * h));
    float hot = clamp((u.exposure - 1.6) / 6., 0., 1.);
    float3 glow = mix(float3(1.0, 0.46, 0.17), float3(1.0, 0.80, 0.62), hot);
    c += u.bloom * breathe * (0.55 * near + 0.30 * far) * glow;

    // The body: a gradient from a pale top to an ember base, darker and more
    // saturated away from the rim, a bright rim at the outline, and a soft
    // highlight near the top; tone-mapped by the exposure.
    if (u.mark > 0. && dpx < 2.) {
        float depth = max(-d, 0.);
        float v = clamp((p.y + 55.) / 112., 0., 1.);
        float3 top = float3(1.00, 0.84, 0.64);
        float3 mid = float3(1.00, 0.50, 0.18);
        float3 base = float3(0.78, 0.22, 0.07);
        float3 body = v < 0.45 ? mix(top, mid, v / 0.45) : mix(mid, base, (v - 0.45) / 0.55);
        body *= mix(1.0, 0.62, smoothstep(0., 16., depth));
        float rim = exp(-depth / 2.6);
        body += rim * float3(1.0, 0.86, 0.70) * 0.95;
        float2 sp = (p - float2(-2., -26.)) / float2(16., 24.);
        body += 0.42 * exp(-dot(sp, sp)) * float3(1.0, 0.93, 0.84);
        body *= 1.55 * (0.96 + 0.04 * breathe);
        float3 lit = 1. - exp(-body * u.exposure);
        float cover = clamp(0.5 - dpx / max(u.density, 1.), 0., 1.) * u.mark;
        c = mix(c, lit, cover);
    }

    float3 o = float3(encode(c.r), encode(c.g), encode(c.b));
    // Film grain at 24 frames a second, about a point across.
    float2 g = floor(px / max(u.density, 1.));
    float n = hash(g + floor(u.time * 24.) * float2(17.13, 31.7)) - 0.5;
    o += n * ((window ? 1.0 : 2.4) / 255.);
    return float4(o, 1.);
}
