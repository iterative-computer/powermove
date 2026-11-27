#include <metal_stdlib>
using namespace metal;
struct Params { float4 rect; float headroom; uint overlay; uint pad0; uint pad1; };
vertex float4 vs(uint i [[vertex_id]]) {
    float2 q=float2(float((i<<1)&2),float(i&2));
    return float4(q*2-1,0,1);
}
float dec(float v) { float m=abs(v); return copysign(m<=0.04045 ? m/12.92 : pow((m+0.055)/1.055,2.4),v); }
fragment half4 fs(float4 p [[position]], texture2d<half,access::read> src [[texture(0)]], constant Params& u [[buffer(0)]]) {
    if(u.overlay==2) {
        const float patches[5]={0.5,1,2,4,8};
        float v=patches[min(uint(p.x/float(src.get_width())*5),4u)];
        return half4(half3(v),1.h);
    }
    float4 c=float4(src.read(uint2(p.xy)));
    float3 v=c.a>0 ? c.rgb/c.a : float3(0);
    v=float3(dec(v.r),dec(v.g),dec(v.b));
    v=float3(0.8224621*v.r+0.1775380*v.g,0.0331941*v.r+0.9668058*v.g,0.0170827*v.r+0.0723974*v.g+0.9105199*v.b);
    // Preserve signed, extended linear engine output. Clipping to a transient
    // headroom query broke viewer=export (especially wide-gamut PQ fixtures).
    // WindowServer maps this labelled EDR surface to the physical display;
    // a mastering transform must be an explicit engine policy, never this clamp.
    if(u.overlay==1) {
        float2 uv=p.xy/float2(src.get_width(),src.get_height());
        float2 d=abs(uv-(u.rect.xy+u.rect.zw)*0.5)-(u.rect.zw-u.rect.xy)*0.5;
        if(max(d.x,d.y)<0.002 && max(d.x,d.y)>-0.002) return half4(0.2h,0.8h,0.45h,1.h);
    }
    return half4(half3(v*c.a),half(c.a));
}
