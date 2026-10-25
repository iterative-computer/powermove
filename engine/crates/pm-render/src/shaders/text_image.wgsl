fn image_texel(p: vec2<i32>) -> vec4<f32> {
    let size = textureDimensions(src, 0);
    if (p.x < 0 || p.y < 0 || p.x >= i32(size.x) || p.y >= i32(size.y)) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, p, 0);
}
fn image_bilinear(u: vec2<f32>) -> vec4<f32> {
    let p = u - vec2<f32>(0.5);
    let lo = vec2<i32>(floor(p));
    let t = fract(p);
    let top = mix(image_texel(lo), image_texel(lo + vec2<i32>(1, 0)), t.x);
    let bottom = mix(image_texel(lo + vec2<i32>(0, 1)), image_texel(lo + vec2<i32>(1, 1)), t.x);
    return mix(top, bottom, t.y);
}

struct Polygon {
    vertices: array<vec2<f32>, 8>,
    count: u32,
};

fn clip_polygon(input: Polygon, axis: u32, bound: f32, keep_above: bool) -> Polygon {
    var output: Polygon;
    output.count = 0u;
    if (input.count == 0u) {
        return output;
    }
    for (var i = 0u; i < input.count; i++) {
        let a = input.vertices[i];
        let b = input.vertices[(i + 1u) % input.count];
        let av = select(a.y, a.x, axis == 0u);
        let bv = select(b.y, b.x, axis == 0u);
        let a_inside = select(av <= bound, av >= bound, keep_above);
        let b_inside = select(bv <= bound, bv >= bound, keep_above);
        if (a_inside != b_inside) {
            let t = (bound - av) / (bv - av);
            output.vertices[output.count] = a + t * (b - a);
            output.count++;
        }
        if (b_inside) {
            output.vertices[output.count] = b;
            output.count++;
        }
    }
    return output;
}

fn polygon_area(poly: Polygon) -> f32 {
    if (poly.count < 3u) {
        return 0.0;
    }
    // Local triangle coordinates keep tiny clipped slivers accurate even
    // when their source texel lies far from the image origin.
    let origin = poly.vertices[0];
    var twice = 0.0;
    for (var i = 1u; i + 1u < poly.count; i++) {
        let a = poly.vertices[i] - origin;
        let b = poly.vertices[i + 1u] - origin;
        twice += a.x * b.y - b.x * a.y;
    }
    return abs(twice) * 0.5;
}

fn image_area(pixel: vec2<f32>) -> vec4<f32> {
    var quad: Polygon;
    quad.count = 4u;
    let corners = array<vec2<f32>, 4>(pixel, pixel + vec2<f32>(1.0, 0.0),
        pixel + vec2<f32>(1.0, 1.0), pixel + vec2<f32>(0.0, 1.0));
    for (var i = 0u; i < 4u; i++) {
        let p = vec3<f32>(corners[i], 1.0);
        quad.vertices[i] = vec2<f32>(dot(it.inv0.xyz, p), dot(it.inv1.xyz, p));
    }
    let whole = polygon_area(quad);
    if (whole <= 0.0) {
        return vec4<f32>(0.0);
    }
    var lo = quad.vertices[0];
    var hi = lo;
    for (var i = 1u; i < 4u; i++) {
        lo = min(lo, quad.vertices[i]);
        hi = max(hi, quad.vertices[i]);
    }
    let size = vec2<i32>(textureDimensions(src, 0));
    let begin = max(vec2<i32>(floor(lo)), vec2<i32>(0));
    let end = min(vec2<i32>(ceil(hi)), size);
    var sum = vec4<f32>(0.0);
    for (var sy = begin.y; sy < end.y; sy++) {
        for (var sx = begin.x; sx < end.x; sx++) {
            var clipped = clip_polygon(quad, 0u, f32(sx), true);
            clipped = clip_polygon(clipped, 0u, f32(sx + 1), false);
            clipped = clip_polygon(clipped, 1u, f32(sy), true);
            clipped = clip_polygon(clipped, 1u, f32(sy + 1), false);
            let weight = polygon_area(clipped) / whole;
            sum += image_texel(vec2<i32>(sx, sy)) * weight;
        }
    }
    return sum;
}

fn paint_image(pos: vec4<f32>) -> vec4<f32> {
    if (it.params.y > 0.5) {
        return image_area(floor(pos.xy)) * it.color.w;
    }
    let p = vec3<f32>(pos.xy, 1.0);
    let u = vec2<f32>(dot(it.inv0.xyz, p), dot(it.inv1.xyz, p));
    return image_bilinear(u) * it.color.w;
}
