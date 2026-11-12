#include <metal_stdlib>
using namespace metal;
struct Line { float4 ends; float4 color; };
struct Params { uint count; uint reduced; uint whole; uint pad2; float4 output; float4 canvas; float2 drawable; float2 pad3; Line lines[120]; };
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
    if(u.x==0||u.x==3) return p;
    float3 c=float3(0.753833*p.r+0.198597*p.g+0.047570*p.b,0.045744*p.r+0.941777*p.g+0.012479*p.b,-0.001210*p.r+0.017602*p.g+0.983609*p.b)*u.w;
    c=float3(eetf(c.r,u.y,u.z),eetf(c.g,u.y,u.z),eetf(c.b,u.y,u.z));
    if(u.x==1) return float3(pq(c.r),pq(c.g),pq(c.b));
    c=max(c/u.z,0.0); float y=dot(c,float3(0.2627,0.6780,0.0593));
    float gamma=1.2+0.42*log10(u.z/1000.0);
    c*=y>0?pow(y,1.0/gamma-1.0):0;
    return float3(hlg(c.r),hlg(c.g),hlg(c.b));
}
// A reduced-resolution frame scales to the drawable once, here; a full one is
// read pixel for pixel, so paused frames equal the export.
constexpr sampler reduced(filter::linear,address::clamp_to_edge);
// A source larger than the drawable (a paused frame rendered at the
// composition's size) is averaged over each drawable pixel's footprint:
// up to 6×6 bilinear taps, premultiplied. A smaller one is sampled once.
half4 scaled(texture2d<half> src,float2 p,float2 drawable) {
    float2 ratio=float2(src.get_width(),src.get_height())/drawable;
    int2 k=int2(clamp(ceil(ratio),float2(1.0),float2(6.0)));
    if(k.x==1&&k.y==1) return src.sample(reduced,p/drawable);
    float2 corner=p-0.5;
    float4 sum=float4(0);
    for(int j=0;j<k.y;j++) for(int i=0;i<k.x;i++) {
        float2 q=corner+(float2(i,j)+0.5)/float2(k);
        sum+=float4(src.sample(reduced,q/drawable));
    }
    return half4(sum/float(k.x*k.y));
}
fragment half4 fs(float4 p [[position]], texture2d<half> src [[texture(0)]], constant Params& u [[buffer(0)]]) {
    // A rendered (disk cache) frame is the whole composition: the drawable
    // samples its window of the canvas.
    float4 c=float4(u.whole!=0 ? src.sample(reduced,(u.canvas.xy+p.xy)/u.canvas.zw)
        : u.reduced!=0 ? scaled(src,p.xy,u.drawable) : src.read(uint2(p.xy)));
    float3 v=c.a>0 ? c.rgb/c.a : float3(0);
    v=float3(dec(v.r),dec(v.g),dec(v.b));
    // An SDR project's view (mode 3) shows its delivery: light above white and
    // colour outside Rec.709 clip as the SDR file clips them. EDR, PQ and HLG
    // keep the extended signal.
    if(u.output.x==3) v=clamp(v,0.0,1.0);
    v=float3(0.8224621*v.r+0.1775380*v.g,0.0331941*v.r+0.9668058*v.g,0.0170827*v.r+0.0723974*v.g+0.9105199*v.b);
    // Preserve signed, extended linear engine output. Clipping to a transient
    // headroom query broke viewer=export (especially wide-gamut PQ fixtures).
    // WindowServer maps this labelled EDR surface to the physical display;
    // a mastering transform must be an explicit engine policy, never this clamp.
    for(uint i=0;i<u.count;i++) {
        // Overlays are normalized to the whole canvas, cropped or not.
        float2 a=u.lines[i].ends.xy*u.canvas.zw-u.canvas.xy,b=u.lines[i].ends.zw*u.canvas.zw-u.canvas.xy;
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
