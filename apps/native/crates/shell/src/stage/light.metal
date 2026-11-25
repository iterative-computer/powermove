// The first-run stage's room and the Powermove mark as a lit object: one
// full-surface pass. The mark is a signed distance field (mark units); its
// body is a warm gradient with a bright rim, tone-mapped by an exposure that
// starts white-hot; its bloom and the light it spills on the stage are
// added around it, over the room's slowly drifting field of silk-like light.
// Output is display-encoded sRGB (BGRA8Unorm).
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
    packed_float3 extra;  // x: the outline's softness (px), y: the body's colour
                          // drift (0–1), z: the room's light field (0–1)
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

static float vnoise(float2 p) {
    float2 i = floor(p), f = fract(p);
    f = f * f * (3. - 2. * f);
    float a = hash(i), b = hash(i + float2(1., 0.)), c = hash(i + float2(0., 1.)), d = hash(i + float2(1., 1.));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
static float fbm(float2 p) {
    float v = 0., a = 0.5;
    const float2x2 turn = float2x2(1.6, 1.2, -1.2, 1.6);
    for (int i = 0; i < 4; i++) {
        v += a * vnoise(p);
        p = turn * p;
        a *= 0.5;
    }
    return v;
}
// The room's light: folds of light like silk or an aurora. A smooth,
// slowly drifting warp bends long diagonal folds; each fold is shaded as a
// surface (a sheen on its crest, plum in its trough). Two layers for depth:
// a broad slow one behind a finer brighter one. Warm only.
static float3 silk(float2 uv, float t) {
    const float3 trough = float3(0.018, 0.004, 0.009);
    const float3 cloth = float3(0.40, 0.068, 0.028);
    const float3 rose = float3(0.40, 0.040, 0.075);
    const float3 sheen = float3(1.00, 0.60, 0.26);
    float3 col = float3(0.);
    for (int k = 0; k < 2; k++) {
        float scale = k == 0 ? 0.55 : 0.95;
        float speed = k == 0 ? 0.018 : 0.032;
        float2 w = uv * scale + float2(float(k) * 4.1, float(k) * 1.7);
        // The warp: two octaves only, so the folds stay long and smooth.
        float2 q = float2(vnoise(w + float2(t * speed, 0.)) + 0.5 * vnoise(w * 2.03 - float2(0., t * speed)),
                          vnoise(w + float2(3.1, 7.7) - float2(0., t * speed * 0.7))
                              + 0.5 * vnoise(w * 1.97 + float2(5.3, t * speed)));
        float phase = (uv.x * 0.55 + uv.y * 1.25) * (k == 0 ? 2.4 : 4.2) + q.x * 3.4 + q.y * 1.6 + t * 0.06;
        float fold = sin(phase * 3.14159);
        float crest = pow(max(fold, 0.), k == 0 ? 2.5 : 7.);
        float lit = pow(0.5 + 0.5 * fold, 1.6);
        float3 tone = mix(trough, mix(cloth, rose, smoothstep(0.6, 1.3, q.y)), lit);
        col += (k == 0 ? 0.75 : 0.55) * (tone + crest * sheen * (k == 0 ? 0.22 : 0.32));
    }
    return col;
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
        // The room's own light: the silk field, lit from above and falling
        // to the floor, at the house lights' level.
        float2 uv = (px - u.size * 0.5) / h;
        float above = smoothstep(0.7, -0.5, uv.y) * (1. - 0.4 * smoothstep(0.5, 1.1, abs(uv.x)));
        c += u.extra.z * u.atmosphere * (0.15 + 0.85 * above) * silk(uv, u.time);
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
    float hot = clamp((u.exposure - 1.6) / 6., 0., 1.);
    // White-hot, the light leads: a wider glow around a soft outline.
    float far = exp(-outside / ((0.075 + 0.09 * hot) * h));
    float3 glow = mix(float3(1.0, 0.46, 0.17), float3(1.0, 0.80, 0.62), hot);
    c += u.bloom * breathe * (0.55 * near + 0.30 * far) * glow;

    // The body: a gradient from a pale top to an ember base, darker and more
    // saturated away from the rim, a bright rim at the outline, and a soft
    // highlight near the top; tone-mapped by the exposure.
    if (u.mark > 0. && dpx < max(2., u.extra.x)) {
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
        // Living colour once landed: a slow warm drift through rose, gold
        // and peach inside the body, never at the rim.
        float drift = u.time * 0.45 + p.x * 0.016 - p.y * 0.011;
        float3 tint = mix(mix(float3(1.0, 0.62, 0.62), float3(1.0, 0.86, 0.50), 0.5 + 0.5 * sin(drift)),
                          float3(1.0, 0.70, 0.42), 0.5 + 0.5 * sin(drift * 0.73 + 2.1));
        body = mix(body, body * tint * 1.25, 0.45 * u.extra.y * (1. - rim));
        body *= 1.55 * (0.96 + 0.04 * breathe);
        float3 lit = 1. - exp(-body * u.exposure);
        float edgeWidth = max(max(u.density, 1.), u.extra.x);
        float cover = clamp(0.5 - dpx / edgeWidth, 0., 1.) * u.mark;
        c = mix(c, lit, cover);
    }

    float3 o = float3(encode(c.r), encode(c.g), encode(c.b));
    // Film grain at 24 frames a second, about a point across.
    float2 g = floor(px / max(u.density, 1.));
    float n = hash(g + floor(u.time * 24.) * float2(17.13, 31.7)) - 0.5;
    o += n * ((window ? 1.0 : 2.4) / 255.);
    return float4(o, 1.);
}
