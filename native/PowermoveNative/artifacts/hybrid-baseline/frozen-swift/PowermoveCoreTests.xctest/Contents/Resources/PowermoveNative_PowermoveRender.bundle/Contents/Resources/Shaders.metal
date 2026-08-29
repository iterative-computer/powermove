#include <metal_stdlib>
using namespace metal;
struct Vertex { float2 position; float2 uv; float4 color; };
struct Varying { float4 position [[position]]; float2 uv; float4 color; };
vertex Varying sceneVertex(uint id [[vertex_id]], const device Vertex* vertices [[buffer(0)]], constant float2 &resolution [[buffer(1)]]) {
    Vertex v = vertices[id];
    return { float4(v.position.x / resolution.x * 2 - 1, 1 - v.position.y / resolution.y * 2, 0, 1), v.uv, v.color };
}
fragment float4 solidFragment(Varying in [[stage_in]]) { return float4(in.color.rgb * in.color.a, in.color.a); }
fragment float4 textureFragment(Varying in [[stage_in]], texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    return tex.sample(s, in.uv) * in.color.a;
}
vertex Varying presentVertex(uint id [[vertex_id]]) {
    float2 uv = float2((id << 1) & 2, id & 2);
    return { float4(uv.x * 2 - 1, 1 - uv.y * 2, 0, 1), uv, float4(1) };
}
fragment float4 presentFragment(Varying in [[stage_in]], texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(filter::nearest, address::clamp_to_edge);
    return tex.sample(s, in.uv);
}
