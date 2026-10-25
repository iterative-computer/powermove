// The direct RG32Float source contains glyph-local points,
// row table entries and pairs of point indices. All integer metadata stays
// below 2^24, where f32 represents it exactly.
fn direct_texel(index: u32) -> vec2<f32> {
    let width = u32(it.inv0.w);
    return textureLoad(src, vec2<i32>(i32(index % width), i32(index / width)), 0).xy;
}

// Exact integral of clamp(linear(t), 0, 1) over t in [0, 1]. The
// monotone segment spends its span below zero, inside [0, 1], or above one.
// Integrating those intervals directly avoids cuts, sorting and loops.
fn direct_clamped_integral(u0: f32, u1: f32) -> f32 {
    let lo = min(u0, u1);
    let hi = max(u0, u1);
    if (hi <= 0.0) { return 0.0; }
    if (lo >= 1.0) { return 1.0; }
    let span = hi - lo;
    if (span == 0.0) { return clamp(lo, 0.0, 1.0); }
    let a = max(lo, 0.0);
    let b = min(hi, 1.0);
    let middle = max(b - a, 0.0) * (0.5 * (a + b));
    let tail = max(hi - max(lo, 1.0), 0.0);
    return (middle + tail) / span;
}

// Green's theorem on a pixel square: each oriented edge contributes
// integral_y clamp(x_edge(y) - pixel_x, 0, 1) dy. The CPU cache has already
// resolved nonzero contours into a disjoint boundary, including holes.
fn direct_area(pixel: vec2<f32>) -> f32 {
    let local = pixel - it.map.xy - it.map.zw;
    let row = i32(floor(local.y));
    let table_row = row - i32(it.inv0.z);
    if (table_row < 0 || table_row >= i32(it.inv1.x)) {
        return 0.0;
    }
    let entry = direct_texel(u32(it.inv0.x) + u32(table_row));
    let begin = u32(entry.x);
    let count = u32(entry.y);
    var area = 0.0;
    for (var k = 0u; k < count; k++) {
        let position = begin + k;
        let pair = direct_texel(u32(it.inv0.y) + position / 2u);
        let edge = u32(select(pair.x, pair.y, (position & 1u) != 0u));
        let a = direct_texel(edge);
        let b = direct_texel(edge + 1u);
        let dy = b.y - a.y;
        if (dy == 0.0) {
            continue;
        }
        let at0 = (local.y - a.y) / dy;
        let at1 = (local.y + 1.0 - a.y) / dy;
        let lo = max(0.0, min(at0, at1));
        let hi = min(1.0, max(at0, at1));
        if (lo >= hi) {
            continue;
        }
        let p = mix(a, b, lo);
        let q = mix(a, b, hi);
        area += (q.y - p.y) * direct_clamped_integral(p.x - local.x, q.x - local.x);
    }
    return clamp(abs(area), 0.0, 1.0);
}

fn paint_outline(pos: vec4<f32>) -> vec4<f32> {
    let kind = it.params.x;
    if (kind < 0.5) {
        let t = vec2<i32>(floor(pos.xy) + it.map.xy);
        let coverage = textureLoad(src, t, 0);
        let c = clamp(coverage.r, 0.0, 1.0);
        return it.color * c;
    }
    if (kind > 2.5) {
        return it.color * direct_area(floor(pos.xy));
    }
    let t = vec2<i32>(floor(pos.xy) + it.map.xy);
    return textureLoad(src, t, 0);
}
