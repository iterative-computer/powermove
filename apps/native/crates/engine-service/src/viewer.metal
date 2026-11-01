#include <metal_stdlib>
using namespace metal;
struct Line { float4 ends; float4 color; };
struct Params { uint count; uint pad0; uint pad1; uint pad2; Line lines[64]; };
vertex float4 vs(uint i [[vertex_id]]) {
    float2 q=float2(float((i<<1)&2),float(i&2));
    return float4(q*2-1,0,1);
}
float dec(float v) { float m=abs(v); return copysign(m<=0.04045 ? m/12.92 : pow((m+0.055)/1.055,2.4),v); }
fragment half4 fs(float4 p [[position]], texture2d<half,access::read> src [[texture(0)]], constant Params& u [[buffer(0)]]) {
    float4 c=float4(src.read(uint2(p.xy)));
    float3 v=c.a>0 ? c.rgb/c.a : float3(0);
    v=float3(dec(v.r),dec(v.g),dec(v.b));
    v=float3(0.8224621*v.r+0.1775380*v.g,0.0331941*v.r+0.9668058*v.g,0.0170827*v.r+0.0723974*v.g+0.9105199*v.b);
    // Preserve signed, extended linear engine output. Clipping to a transient
    // headroom query broke viewer=export (especially wide-gamut PQ fixtures).
    // WindowServer maps this labelled EDR surface to the physical display;
    // a mastering transform must be an explicit engine policy, never this clamp.
    for(uint i=0;i<u.count;i++) {
        float2 size=float2(src.get_width(),src.get_height());
        float2 a=u.lines[i].ends.xy*size,b=u.lines[i].ends.zw*size;
        float2 ab=b-a;
        float t=clamp(dot(p.xy-a,ab)/max(dot(ab,ab),0.0001),0.0,1.0);
        if(length(p.xy-(a+t*ab))<=1.0) {
            float4 o=u.lines[i].color;
            return half4(half3(o.rgb*o.a+v*c.a*(1-o.a)),half(o.a+c.a*(1-o.a)));
        }
    }
    return half4(half3(v*c.a),half(c.a));
}
