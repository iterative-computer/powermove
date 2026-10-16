// Display geometry of a converted source (footage.rs, `Geometry`): the
// clean aperture crop, then quarter turns counterclockwise. Every output
// pixel copies one input pixel; nothing is resampled. `OUT_FORMAT` is
// replaced with the working storage format.

struct Params {
    // crop x, y, w, h (coded pixels).
    crop: vec4<u32>,
    // quarter turns, output width, output height.
    turn: vec4<u32>,
};

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var<uniform> P: Params;
@group(0) @binding(2) var out: texture_storage_2d<OUT_FORMAT, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.turn.y || id.y >= P.turn.z) {
        return;
    }
    let cw = P.crop.z;
    let ch = P.crop.w;
    var uv: vec2<u32>;
    switch P.turn.x {
        case 1u: {
            uv = vec2<u32>(cw - 1u - id.y, id.x);
        }
        case 2u: {
            uv = vec2<u32>(cw - 1u - id.x, ch - 1u - id.y);
        }
        case 3u: {
            uv = vec2<u32>(id.y, ch - 1u - id.x);
        }
        default: {
            uv = id.xy;
        }
    }
    let v = textureLoad(input, vec2<i32>(P.crop.xy + uv), 0);
    textureStore(out, vec2<i32>(id.xy), v);
}
