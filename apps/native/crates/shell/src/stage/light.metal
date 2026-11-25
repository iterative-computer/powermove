// The first-run stage's room and the Powermove mark as a lit object: one
// full-surface pass. The mark is a signed distance field (mark units); its
// body is a warm gradient with a bright rim, tone-mapped by an exposure that
// starts white-hot; its bloom and the light it spills on the stage are
// added around it, over the room's slowly drifting bands of light.
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
// The room's light: bands of light on near-black. Long bands sweep across
// the room on a slow warp, each lit hard on its leading edge (a crisp line
// of sheen) and fading across its width, with a faint glow around it; a
// nearer layer over a dimmer one behind, adding where they cross, like
// light. Warm only: ember, rose and gold.
static float3 band(float2 uv, float t, float k) {
    const float3 ember = float3(0.95, 0.22, 0.045);
    const float3 rose = float3(0.85, 0.10, 0.17);
    const float3 gold = float3(1.00, 0.56, 0.16);
    const float3 sheen = float3(1.00, 0.80, 0.56);
    float freq = k < 0.5 ? 2.1 : 3.3;
    float speed = k < 0.5 ? 0.020 : 0.034;
    float2 dir = normalize(k < 0.5 ? float2(0.42, 1.0) : float2(0.62, 1.0));
    float2 w = uv * (k < 0.5 ? 0.42 : 0.60) + float2(k * 5.3, k * 2.1);
    // A gentle warp: one smooth octave and a half, so the bands stay long,
    // clean curves.
    float bend = vnoise(w + float2(t * speed, -t * speed * 0.6)) - 0.5
        + 0.35 * (vnoise(w * 1.9 + float2(-t * speed, 4.1)) - 0.5);
    float phase = dot(uv, dir) * freq + bend * 1.35 + t * speed * 1.4;
    float cell = floor(phase), f = fract(phase);
    float r = hash(float2(cell, 7.3 + k * 13.1));
    float r2 = hash(float2(cell * 1.7, 2.9 + k * 5.7));
    // A pixel of phase, so the edges stay a pixel wide at any size.
    float px = max(fwidth(phase), 1e-4);
    float width = 0.14 + 0.30 * r2;
    float on = step(0.22, r2);
    float lead = smoothstep(0., 1.4 * px, f);
    float tail = 1. - smoothstep(width - 0.05, width, f);
    float3 tone = r < 0.38 ? ember : (r < 0.70 ? rose : gold);
    float level = on * (0.30 + 0.70 * r * r);
    // Light across the band: brightest at its edge, fading to its far side.
    float across = mix(1., 0.12, smoothstep(0.18 * width, 0.95 * width, f));
    float3 col = tone * across * lead * tail * level;
    col += 1.6 * sheen * level * lead * exp(-f / (1.5 * px + 0.003));
    // A glow around the band: ahead of its edge and behind its far side.
    float ahead = exp(-(1. - f) / 0.03);
    float behind = exp(-max(f - width, 0.) / 0.05) * (1. - tail);
    col += tone * level * 0.10 * behind;
    float nextOn = step(0.22, hash(float2((cell + 1.) * 1.7, 2.9 + k * 5.7)));
    float nr = hash(float2(cell + 1., 7.3 + k * 13.1));
    float3 ntone = nr < 0.38 ? ember : (nr < 0.70 ? rose : gold);
    col += ntone * nextOn * (0.30 + 0.70 * nr * nr) * 0.22 * ahead;
    return col;
}
static float3 room_field(float2 uv, float t) {
    return 0.55 * band(uv, t, 0.) + 1.05 * band(uv + float2(0.37, -0.21), t, 1.);
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
    // White-hot, the outline swells and rounds (the field dilated by most of
    // its softness): the light rises as a dome, never as a thin white spike,
    // and resolves to the mark as the exposure comes down.
    float dpx = d * u.scale - 0.8 * u.extra.x;

    // The room: near-black under its bands of light. A
    // window: its own ground, a touch lighter under its top edge.
    float3 c;
    float3 ground = float3(u.ground);
    bool window = any(ground > 0.);
    if (window) {
        float lift = exp(-px.y / (0.5 * h));
        c = ground * (1. + 0.10 * lift);
    } else {
        // The room's own light: bands on near-black, lit from above and
        // falling to the floor, at the house lights' level.
        float2 uv = (px - u.size * 0.5) / h;
        float above = smoothstep(0.75, -0.6, uv.y);
        above *= (1. - 0.35 * smoothstep(0.5, 1.1, abs(uv.x)));
        c = float3(0.0016, 0.0010, 0.0013) + u.extra.z * u.atmosphere * (0.08 + 0.92 * above)
            * room_field(uv, u.time);
    }

    // Breathing: a slow swell of the light, never a pulse.
    float breathe = 1. + 0.07 * sin(u.time * 6.2831853 / 3.4);
    float markH = u.reach;

    // The light on the stage under the mark: a wide, low pool.
    float2 pq = (px - float2(u.centre.x, u.centre.y + 0.78 * markH)) / float2(1.9 * markH, 0.42 * markH);
    c += u.pool * breathe * 0.075 * exp(-dot(pq, pq)) * float3(1.0, 0.52, 0.24);
    // Its reach into the room: a broad halo, faint.
    float2 hq = (px - u.centre) / (2.0 * markH);
    c += u.pool * breathe * 0.010 * exp(-dot(hq, hq)) * float3(1.0, 0.58, 0.32);

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
    // Kept close and warm, so the room around stays dark: the mark is the light.
    float far = exp(-outside / ((0.060 + 0.035 * hot) * h));
    float3 glow = mix(float3(1.0, 0.46, 0.17), float3(1.0, 0.68, 0.42), hot);
    c += u.bloom * breathe * (0.55 * near + 0.20 * far) * glow;

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
