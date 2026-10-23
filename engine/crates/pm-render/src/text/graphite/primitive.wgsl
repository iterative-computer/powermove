struct Params { data: array<vec4<f32>,9> };
@group(0) @binding(0) var<uniform> p: Params;
@vertex fn primitive_vs(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(p.data[0].zw*pos-sign(p.data[0].zw),0.5,1.0);
}
@fragment fn primitive_fs() -> @location(0) vec4<f32> { return p.data[8]; }
