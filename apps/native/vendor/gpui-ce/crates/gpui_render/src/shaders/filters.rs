#[wgsl_rs::wgsl]
pub mod surface {
    use super::super::common::*;
    use super::super::corner_smoothing::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct SurfaceUniforms {
        pub bounds: Bounds,
        pub content_mask: ContentMask,
        pub color_format: SurfaceColorFormat,
        pub opacity: f32,
        pub padding0: u32,
        pub padding1: u32,
        pub padding2: u32,
        pub padding3: u32,
        pub padding4: u32,
        pub padding5: u32,
    }
    uniform!(group(1), binding(0), SURFACE_LOCALS: SurfaceUniforms);
    texture!(group(1), binding(1), SURFACE_TEXTURE: Texture2D<f32>);
    texture!(group(1), binding(2), SURFACE_CHROMA_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(3), SURFACE_SAMPLER: Sampler);

    pub const YCBCR_TO_LINEAR_RGB: Mat4x4f = mat4x4f(
        vec4f(1.0000, 1.0000, 1.0000, 0.0),
        vec4f(0.0000, -0.3441, 1.7720, 0.0),
        vec4f(1.4020, -0.7141, 0.0000, 0.0),
        vec4f(-0.7010, 0.5291, -0.8860, 1.0),
    );

    pub fn sample_yuv_surface(texture_position: Vec2f) -> Vec4f {
        let luma = texture_sample_level(SURFACE_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).x;
        let chroma = texture_sample_level(
            SURFACE_CHROMA_TEXTURE,
            SURFACE_SAMPLER,
            texture_position,
            0.0,
        )
        .xy();
        YCBCR_TO_LINEAR_RGB * vec4f(luma, chroma.x, chroma.y, 1.0)
    }

    #[derive(Wgsl)]
    pub struct SurfaceVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_position: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_surface(#[builtin(vertex_index)] vertex_id: u32) -> SurfaceVarying {
        let vertex = rectangle_vertex(vertex_id, get!(SURFACE_LOCALS).bounds);
        SurfaceVarying {
            position: vertex.clip_position,
            texture_position: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(SURFACE_LOCALS).content_mask.bounds,
            ),
        }
    }

    #[fragment]
    pub fn fragment_surface(input: SurfaceVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let locals = get!(SURFACE_LOCALS);
        let fade = content_mask_alpha(locals.content_mask, input.position.xy());
        if locals.color_format == SurfaceColorFormat::Yuv {
            return sample_yuv_surface(input.texture_position) * locals.opacity * fade;
        }
        texture_sample_level(
            SURFACE_TEXTURE,
            SURFACE_SAMPLER,
            input.texture_position,
            0.0,
        ) * locals.opacity
            * fade
    }
}

#[wgsl_rs::wgsl]
pub mod blur {
    use super::super::common::*;
    use super::super::corner_smoothing::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurUniforms {
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub corner_radii: Corners,
        pub direction: Vec2f,
        pub standard_deviation: f32,
        pub opacity: f32,
        pub sample_count: u32,
        pub sample_step: f32,
        pub composite_clip: BlurCompositeClip,
        pub downsample_mode: DownsampleMode,
        pub source_size: Vec2f,
        pub target_size: Vec2f,
        pub corner_smoothing: f32,
        pub padding0: u32,
        pub padding1: u32,
        pub padding2: u32,
        pub dots: Vec4f,
        // DotMatrix: normalized accent RGB. DotAurora: three packed RGB24 stops and fade.
        pub dot_accent: Vec4f,
    }
    uniform!(group(1), binding(0), BLUR_LOCALS: BlurUniforms);
    texture!(group(1), binding(1), BLUR_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), BLUR_SAMPLER: Sampler);

    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurCompositeVertexData {
        pub position: Vec4f,
        pub texture_coordinates: Vec2f,
        pub clip_distances: Vec4f,
    }

    pub fn prepare_blur_composite_vertex(vertex_id: u32) -> BlurCompositeVertexData {
        let vertex = rectangle_vertex(vertex_id, get!(BLUR_LOCALS).bounds);
        BlurCompositeVertexData {
            position: vertex.clip_position,
            texture_coordinates: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(BLUR_LOCALS).content_mask,
            ),
        }
    }

    pub fn dot_hash(p: Vec2f) -> f32 {
        fract(sin(dot(p, vec2f(12.9898, 78.233))) * 43758.5453)
    }

    pub fn dot_noise(p: Vec2f) -> f32 {
        let i = floor(p);
        let f = p - i;
        let u = f * f * (vec2f(3.0, 3.0) - f * 2.0);
        let a = dot_hash(i);
        let b = dot_hash(i + vec2f(1.0, 0.0));
        let c = dot_hash(i + vec2f(0.0, 1.0));
        let d = dot_hash(i + vec2f(1.0, 1.0));
        mix(mix(a, b, u.x), mix(c, d, u.x), u.y)
    }

    pub fn dot_fbm(p: Vec2f) -> f32 {
        let mut q = p;
        let mut v = 0.0;
        let mut amp = 0.5;
        for i in 0..3 {
            v = v + amp * dot_noise(q);
            q = q * 2.03 + vec2f(11.7 + 0.0 * (i as f32), 11.7);
            amp = amp * 0.5;
        }
        v
    }

    pub fn unpack_aurora_color(packed: f32) -> Vec3f {
        let rgb = packed as u32;
        vec3f(
            ((rgb >> 16u32) & 255u32) as f32,
            ((rgb >> 8u32) & 255u32) as f32,
            (rgb & 255u32) as f32,
        ) / 255.0
    }

    pub fn blur_composite_color(position: Vec2f, coverage: f32) -> Vec4f {
        let params = get!(BLUR_LOCALS).dots;
        // GPU-rendered dot-matrix aurora (params.x < 0.0).
        // Zero CPU quads, zero texture samples.
        if params.w > 0.0 && params.x < 0.0 {
            let scale = params.w;
            let age = params.y;
            let opening = params.z;
            let palette = get!(BLUR_LOCALS).dot_accent;
            let size = max(get!(BLUR_LOCALS).bounds.size / scale, vec2f(1.0, 1.0));
            let local = (position - get!(BLUR_LOCALS).bounds.origin) / scale;
            let cell = floor((local - vec2f(1.0, 1.0)) / 3.0 + vec2f(0.5, 0.5));
            let center = cell * 3.0 + vec2f(1.0, 1.0);
            if center.x < 1.0 || center.y < 1.0 || center.x > size.x || center.y > size.y {
                return transparent();
            }
            if center.y > 3.0 + (size.y - 3.0) * opening || dot_hash(center) > opening {
                return transparent();
            }
            let dot_cov = clamp(0.75 - length(local - center) + 0.5 / scale, 0.0, 1.0);
            if dot_cov <= 0.0 {
                return transparent();
            }
            let xn = center.x / size.x * 2.2;
            let up = 1.0 - center.y / size.y;
            let mut swirl = 0.0;
            let mut rise = 0.0;
            for j in 0..4 {
                let f = j as f32;
                let hem = 0.04
                    + 0.25 * f
                    + 0.09 * sin(xn * (1.6 + 0.7 * f) + age * (0.45 + 0.2 * f) + f * 2.1)
                    + 0.09 * (dot_fbm(vec2f(xn * 1.4 + age * 0.12, f * 3.7)) - 0.5);
                let above = up - hem;
                let curtain = select(
                    0.3 * exp(-above * 3.0) + 0.7 * exp(-above * 40.0),
                    exp(max(above * 65.0, -40.0)),
                    above < 0.0,
                );
                let r = clamp(
                    (dot_fbm(vec2f(
                        xn * 20.0 + age * 0.2 + f * 5.0,
                        f * 7.0 + up * 0.025 - age * 0.02,
                    )) - 0.22)
                        / 0.5,
                    0.0,
                    1.0,
                );
                let v = 2.2
                    * curtain
                    * (0.08 + 0.92 * r * r * (3.0 - 2.0 * r))
                    * (0.8 + 0.2 * sin(xn * 11.0 - age * 2.2 + f * 1.3))
                    * (1.0 - 0.1 * f);
                if v > swirl {
                    swirl = v;
                    rise = clamp(above * 4.0, 0.0, 1.0);
                }
            }
            let s = clamp(swirl, 0.0, 1.0);
            let s2 = s * s;
            let dot_alpha = min(0.02 + 0.1 * s + 0.88 * s2 * s2, 1.0);
            let gold = unpack_aurora_color(palette.x);
            let accent = unpack_aurora_color(palette.y);
            let rose = unpack_aurora_color(palette.z);
            let t1 = clamp(rise / 0.35, 0.0, 1.0);
            let t2 = clamp((rise - 0.45) / 0.55, 0.0, 1.0) * 0.45;
            let warm = gold * (1.0 - t1) + accent * t1;
            let color = warm * (1.0 - t2) + rose * t2;
            let a = dot_alpha * dot_cov * palette.w * coverage * get!(BLUR_LOCALS).opacity;
            return vec4f(color.x * a, color.y * a, color.z * a, a);
        }
        let mut blurred = texture_sample_level(
            BLUR_TEXTURE,
            BLUR_SAMPLER,
            position / get!(BLUR_LOCALS).target_size,
            0.0,
        );
        if params.w > 0.0 {
            let scale = params.w;
            let local = (position - get!(BLUR_LOCALS).bounds.origin) / scale;
            let mut best = vec4f(0.0, 0.0, 0.0, 0.0);
            for k in 0..4 {
                let pitch = 16.0 / pow(2.0, k as f32);
                let idx = floor(local / pitch + vec2f(0.5, 0.5));
                if k > 0 && fract(idx.x / 2.0) == 0.0 && fract(idx.y / 2.0) == 0.0 {
                    continue;
                }
                let gp = idx * pitch;
                // Reject pixels outside the dot disk before the 3x3 texture fetch loop.
                let disk = clamp(1.35 - length(local - gp) + 0.5 / scale, 0.0, 1.0);
                if disk <= 0.0 {
                    continue;
                }
                let stagger = fract(sin(dot(gp, vec2f(12.9898, 78.233))) * 43758.5453);
                let vis = clamp((params.x - k as f32 - stagger * 0.85) / 0.15, 0.0, 1.0);
                if vis <= 0.0 {
                    continue;
                }
                let mut sample = vec4f(0.0, 0.0, 0.0, 0.0);
                for y in 0..3 {
                    for x in 0..3 {
                        let offset = vec2f(x as f32 - 1.0, y as f32 - 1.0) * pitch * 0.33;
                        let uv = (get!(BLUR_LOCALS).bounds.origin + (gp + offset) * scale)
                            / get!(BLUR_LOCALS).target_size;
                        sample = sample
                            + texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, uv, 0.0) / 9.0;
                    }
                }
                let cov = clamp(sample.w, 0.0, 1.0);
                let a = disk * vis * smoothstep(0.04, 0.35, cov);
                if a > best.w {
                    let base = sample.xyz() / max(cov, 0.001);
                    let heat = 0.18 + 0.1 * cov;
                    let hot = get!(BLUR_LOCALS).dot_accent.xyz() * (1.0 - heat)
                        + vec3f(1.0, 1.0, 1.0) * heat;
                    let color = min((base * 0.2 + hot * 0.8) * 1.12, vec3f(1.0, 1.0, 1.0));
                    best = vec4f(color.x * a, color.y * a, color.z * a, a);
                }
            }
            blurred = best * (1.0 - params.y) + blurred * params.y;
        }
        let factor = coverage * get!(BLUR_LOCALS).opacity;
        vec4f(
            blurred.x * factor,
            blurred.y * factor,
            blurred.z * factor,
            blurred.w * factor,
        )
    }

    #[derive(Wgsl)]
    pub struct BlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_blur_fullscreen(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = fullscreen_vertex(vertex_id);
        BlurVarying {
            position: vertex.clip_position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: unclipped_distances(),
        }
    }

    #[fragment]
    pub fn fragment_blur_downsample(input: BlurVarying) -> Vec4f {
        if get!(BLUR_LOCALS).downsample_mode == DownsampleMode::HalfResolution {
            let destination = floor(input.position.xy());
            let source_coordinates = min(
                (destination + 0.5) / get!(BLUR_LOCALS).target_size,
                (get!(BLUR_LOCALS).source_size - 0.5) / get!(BLUR_LOCALS).source_size,
            );
            return texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, source_coordinates, 0.0);
        }
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, input.texture_coordinates, 0.0)
    }

    pub fn sample_blur_texture(texture_coordinates: Vec2f) -> Vec4f {
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, texture_coordinates, 0.0)
    }

    pub fn gaussian_blur(texture_coordinates: Vec2f) -> Vec4f {
        let uniforms = get!(BLUR_LOCALS);
        let center_weight = gaussian(0.0, uniforms.standard_deviation);
        let mut weighted_color = sample_blur_texture(texture_coordinates) * center_weight;
        let mut weight_sum = center_weight;
        let mut sample_index = 1u32;

        while sample_index <= uniforms.sample_count {
            let distance = sample_index as f32 * uniforms.sample_step;
            let weight = gaussian(distance, uniforms.standard_deviation);
            let coordinate_offset = uniforms.direction * distance;
            let symmetric_pair = sample_blur_texture(texture_coordinates - coordinate_offset)
                + sample_blur_texture(texture_coordinates + coordinate_offset);
            weighted_color = weighted_color + symmetric_pair * weight;
            weight_sum += 2.0 * weight;
            sample_index += 1u32;
        }
        weighted_color / max(weight_sum, MIN_NORMALIZED_WEIGHT)
    }

    #[fragment]
    pub fn fragment_blur(input: BlurVarying) -> Vec4f {
        gaussian_blur(input.texture_coordinates)
    }

    #[vertex]
    pub fn vertex_blur_composite(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        BlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: vertex.clip_distances,
        }
    }

    #[fragment]
    pub fn fragment_blur_composite(input: BlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let coverage = select(
            1.0,
            antialiased_coverage(rounded_rectangle_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(input.position.xy(), coverage)
    }

    #[derive(Wgsl)]
    pub struct SmoothedBlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub horizontal_corner_reaches: Vec4f,
        #[location(2)]
        #[interpolate(flat)]
        pub vertical_corner_reaches: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
        #[location(4)]
        #[interpolate(flat)]
        pub smoothing_factors: Vec4f,
        #[location(5)]
        #[interpolate(flat)]
        pub superellipse_power: f32,
    }

    #[vertex]
    pub fn vertex_smoothed_blur_composite(
        #[builtin(vertex_index)] vertex_id: u32,
    ) -> SmoothedBlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        let prepared = prepare_corners(
            get!(BLUR_LOCALS).bounds.size,
            get!(BLUR_LOCALS).corner_radii,
            get!(BLUR_LOCALS).corner_smoothing,
            true,
        );
        SmoothedBlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            horizontal_corner_reaches: prepared.horizontal_reaches,
            vertical_corner_reaches: prepared.vertical_reaches,
            clip_distances: vertex.clip_distances,
            smoothing_factors: prepared.smoothing_factors,
            superellipse_power: prepared.superellipse_power,
        }
    }

    #[fragment]
    pub fn fragment_smoothed_blur_composite(input: SmoothedBlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let coverage = select(
            1.0,
            antialiased_coverage(prepared_corner_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
                get!(BLUR_LOCALS).corner_smoothing,
                PreparedCorners {
                    horizontal_reaches: input.horizontal_corner_reaches,
                    vertical_reaches: input.vertical_corner_reaches,
                    smoothing_factors: input.smoothing_factors,
                    superellipse_power: input.superellipse_power,
                },
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(input.position.xy(), coverage)
    }
}
