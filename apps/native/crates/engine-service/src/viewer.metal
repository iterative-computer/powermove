#include <metal_stdlib>
using namespace metal;
struct Line { float4 ends; float4 color; };
struct Params { uint count; uint pad0; uint pad1; uint pad2; float4 output; Line lines[120]; };
vertex float4 vs(uint i [[vertex_id]]) {
    float2 q=float2(float((i<<1)&2),float(i&2));
    return float4(q*2-1,0,1);
}
float dec(float v) { float m=abs(v); return copysign(m<=0.04045 ? m/12.92 : pow((m+0.055)/1.055,2.4),v); }
float pq(float n) { float y=pow(max(n,0.0)/10000.0,2610.0/16384.0); return pow((3424.0/4096.0+2413.0/128.0*y)/(1.0+2392.0/128.0*y),2523.0/32.0); }
float unpq(float e) { float p=pow(max(e,0.0),32.0/2523.0); return 10000.0*pow(max(p-3424.0/4096.0,0.0)/(2413.0/128.0-2392.0/128.0*p),16384.0/2610.0); }
float eetf(float n,float source,float target) {
    if(target>=source) return n;
    float pw=pq(source), m=pq(target)/pw, e=clamp(pq(n)/pw,0.0,1.0), k=1.5*m-0.5;
    if(e<k) return n;
    float t=(e-k)/(1.0-k), t2=t*t,t3=t2*t;
    return unpq(((2*t3-3*t2+1)*k+(t3-2*t2+t)*(1-k)+(-2*t3+3*t2)*m)*pw);
}
float hlg(float l) { return l<=1.0/12.0?sqrt(3.0*max(l,0.0)):0.17883277*log(12.0*l-0.28466892)+0.55991073; }
float3 mastering(float3 p,float4 u) {
    if(u.x==0) return p;
    float3 c=float3(0.753833*p.r+0.198597*p.g+0.047570*p.b,0.045744*p.r+0.941777*p.g+0.012479*p.b,-0.001210*p.r+0.017602*p.g+0.983609*p.b)*u.w;
    c=float3(eetf(c.r,u.y,u.z),eetf(c.g,u.y,u.z),eetf(c.b,u.y,u.z));
    if(u.x==1) return float3(pq(c.r),pq(c.g),pq(c.b));
    c=max(c/u.z,0.0); float y=dot(c,float3(0.2627,0.6780,0.0593));
    float gamma=1.2+0.42*log10(u.z/1000.0);
    c*=y>0?pow(y,1.0/gamma-1.0):0;
    return float3(hlg(c.r),hlg(c.g),hlg(c.b));
}
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
            float a=o.a+c.a*(1-o.a);
            float3 mixed=o.rgb*o.a+v*c.a*(1-o.a);
            return half4(half3(mastering(a>0?mixed/a:float3(0),u.output)*a),half(a));
        }
    }
    return half4(half3(mastering(v,u.output)*c.a),half(c.a));
}
