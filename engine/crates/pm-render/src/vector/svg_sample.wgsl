struct Params {
    x: vec4<f32>,
    y: vec4<f32>,
    size: vec4<f32>,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 2.0 - 1.0, 1.0 - f32(i >> 1u) * 2.0, 0.0, 1.0);
}

fn cross(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// Area of an inverse-mapped output pixel intersecting one source texel.
// The source texel is an area average, so this conserves alpha under motion.
fn cell_integrals(quad: array<vec2<f32>, 4>, cell: vec2<f32>, winding: f32) -> vec3<f32> {
    var entirely_inside = true;
    for (var side = 0u; side < 4u; side++) {
        let a = quad[side];
        let edge = quad[(side + 1u) & 3u] - a;
        let at_min = winding * cross(edge, cell - a);
        let step_x = winding * -edge.y;
        let step_y = winding * edge.x;
        let low = at_min + min(0.0, step_x) + min(0.0, step_y);
        let high = at_min + max(0.0, step_x) + max(0.0, step_y);
        if (high <= 0.0) { return vec3<f32>(0.0); }
        entirely_inside = entirely_inside && low >= 0.0;
    }
    if (entirely_inside) { return vec3<f32>(1.0, 0.0, 0.0); }

    var points: array<vec2<f32>, 8>;
    var next: array<vec2<f32>, 8>;
    for (var i = 0u; i < 4u; i++) { points[i] = quad[i] - cell; }
    var count = 4u;
    for (var side = 0u; side < 4u; side++) {
        if (count == 0u) { return vec3<f32>(0.0); }
        var n = 0u;
        for (var i = 0u; i < count; i++) {
            let a = points[i];
            let b = points[(i + 1u) % count];
            var va = a.x;
            var vb = b.x;
            if (side == 1u) { va = 1.0 - a.x; vb = 1.0 - b.x; }
            if (side == 2u) { va = a.y; vb = b.y; }
            if (side == 3u) { va = 1.0 - a.y; vb = 1.0 - b.y; }
            let inside_a = va >= 0.0;
            let inside_b = vb >= 0.0;
            if (inside_a != inside_b) {
                next[n] = mix(a, b, va / (va - vb));
                n++;
            }
            if (inside_b) { next[n] = b; n++; }
        }
        count = n;
        for (var i = 0u; i < count; i++) { points[i] = next[i]; }
    }
    if (count < 3u) { return vec3<f32>(0.0); }
    var twice = 0.0;
    var moment = vec2<f32>(0.0);
    for (var i = 0u; i < count; i++) {
        let a = points[i];
        let b = points[(i + 1u) % count];
        let wedge = cross(a, b);
        twice += wedge;
        moment += (a + b) * wedge;
    }
    let area = clamp(abs(twice) * 0.5, 0.0, 1.0);
    let first = moment * (sign(twice) / 6.0) - vec2<f32>(0.5 * area);
    return vec3<f32>(area, first);
}

fn limited_slope(left: vec4<f32>, middle: vec4<f32>, right: vec4<f32>) -> vec4<f32> {
    let a = middle - left;
    let b = right - middle;
    return select(vec4<f32>(0.0), sign(a) * min(abs(a), abs(b)), a * b > vec4<f32>(0.0));
}

@fragment fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = floor(position.xy);
    let origin = vec2<f32>(dot(p.x.xyz, vec3<f32>(pixel, 1.0)), dot(p.y.xyz, vec3<f32>(pixel, 1.0)));
    let ex = vec2<f32>(p.x.x, p.y.x);
    let ey = vec2<f32>(p.x.y, p.y.y);
    let quad = array<vec2<f32>, 4>(origin, origin + ex, origin + ex + ey, origin + ey);
    let winding = select(-1.0, 1.0, cross(ex, ey) >= 0.0);
    let covered_source_area = abs(cross(ex, ey));
    if (covered_source_area < 1e-8) { return vec4<f32>(0.0); }
    let minimum = min(min(quad[0], quad[1]), min(quad[2], quad[3]));
    let maximum = max(max(quad[0], quad[1]), max(quad[2], quad[3]));
    if (maximum.x <= 0.0 || maximum.y <= 0.0 || minimum.x >= p.size.x || minimum.y >= p.size.y) {
        return vec4<f32>(0.0);
    }
    let low = max(vec2<i32>(floor(minimum)), vec2<i32>(0));
    let high = min(vec2<i32>(ceil(maximum)), vec2<i32>(p.size.xy));
    let axis_aligned = ex.y == 0.0 && ey.x == 0.0;
    var color = vec4<f32>(0.0);
    for (var y = low.y; y < high.y; y++) {
        for (var x = low.x; x < high.x; x++) {
            var integral = vec3<f32>(0.0);
            if (axis_aligned) {
                let left = max(minimum.x, f32(x)) - f32(x);
                let right = min(maximum.x, f32(x + 1)) - f32(x);
                let top = max(minimum.y, f32(y)) - f32(y);
                let bottom = min(maximum.y, f32(y + 1)) - f32(y);
                let area = max(0.0, right - left) * max(0.0, bottom - top);
                integral = vec3<f32>(area, area * (0.5 * (left + right) - 0.5), area * (0.5 * (top + bottom) - 0.5));
            } else {
                integral = cell_integrals(quad, vec2<f32>(f32(x), f32(y)), winding);
            }
            if (integral.x > 0.0) {
                let center = textureLoad(source, vec2<i32>(x, y), 0);
                color += center * integral.x;
                if (integral.y != 0.0) {
                    let left = textureLoad(source, vec2<i32>(max(x - 1, 0), y), 0);
                    let right = textureLoad(source, vec2<i32>(min(x + 1, i32(p.size.x) - 1), y), 0);
                    color += limited_slope(left, center, right) * integral.y;
                }
                if (integral.z != 0.0) {
                    let top = textureLoad(source, vec2<i32>(x, max(y - 1, 0)), 0);
                    let bottom = textureLoad(source, vec2<i32>(x, min(y + 1, i32(p.size.y) - 1)), 0);
                    color += limited_slope(top, center, bottom) * integral.z;
                }
            }
        }
    }
    return color / covered_source_area;
}
