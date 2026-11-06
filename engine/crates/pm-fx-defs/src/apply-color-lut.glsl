#version 300 es
precision highp float;
in vec2 v_uv; in vec2 v_st; in vec2 v_px;
uniform sampler2D u_tex;
uniform vec2 u_res;
uniform vec2 u_coordRes;     // full composition pixel size for cropped effects
uniform vec2 u_pxOrigin;     // cropped viewport origin in composition pixels
uniform vec2 u_texel;
uniform float u_time;
uniform float u_prog;
out vec4 o;
vec4 src(vec2 uv){ return texture(u_tex, uv); }
float luma(vec3 c){ return dot(c, vec3(.2126,.7152,.0722)); }
vec3 hue2rgb(float h){ return clamp(abs(mod(h*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.); }
vec3 rgb2hsv(vec3 c){
  vec4 K=vec4(0.,-1./3.,2./3.,-1.); vec4 p=mix(vec4(c.bg,K.wz),vec4(c.gb,K.xy),step(c.b,c.g));
  vec4 q=mix(vec4(p.xyw,c.r),vec4(c.r,p.yzx),step(p.x,c.r));
  float d=q.x-min(q.w,q.y); float e=1e-10;
  return vec3(abs(q.z+(q.w-q.y)/(6.*d+e)), d/(q.x+e), q.x);
}
vec3 hsv2rgb(vec3 c){ return c.z*mix(vec3(1.),clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.),c.y); }
float hash(vec2 p){ return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
float noise(vec2 p){
  vec2 i=floor(p), f=fract(p); f=f*f*(3.-2.*f);
  return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);
}
float fbm(vec2 p){ float v=0.,a=.5; for(int i=0;i<5;i++){ v+=a*noise(p); p*=2.03; a*=.5; } return v; }

uniform vec3 u_lutDomainMin,u_lutDomainMax; uniform float u_lutSize;
uniform sampler2D u_lut; uniform float u_has_lut; uniform vec2 u_lutRes;
uniform float u_lutCubeSize,u_lutColumns,u_lutShaperSize,u_lutShaperRow; uniform vec2 u_lutCubeRange;
uniform float u_amount,u_inputSpace,u_outputSpace;
uniform int u_pass;
#define u_p1 u_amount
#define u_p2 u_inputSpace
#define u_p3 u_outputSpace
float lut_e_dec(float v){float m=abs(v);float l=m<=.04045?m/12.92:pow((m+.055)/1.055,2.4);return v<0.?-l:l;}
float lut_e_enc(float l){float m=abs(l);float e=m<=.0031308?12.92*m:1.055*pow(m,1./2.4)-.055;return l<0.?-e:e;}
float lut_pow2(float x){return pow(2.,x);}
float lut_log_enc(float l,int s){
 if(s==2){if(l<-.05641088)return 0.;if(l<.01){float d=l+.05641088;return 47.28711236*d*d;}return .08550479*log2(l+.00964052)+.69336945;}
 if(s==3||s==4){if(l>=.01125)return (420.+log((l+.01)/.19)/log(10.)*261.5)/1023.;return (l*76.2102946929/.01125+95.)/1023.;}
 if(s==5){if(l<.01)return 5.6*l+.125;return .241514*log(l+.00873)/log(10.)+.598206;}
 if(s==6){if(l>.010591)return .24719*log(5.555556*l+.052272)/log(10.)+.385537;return 5.367655*l+.092809;}
 if(s==7){float a=262128./117.45,b=928./1023.,c=95./1023.;float t=(lut_pow2(14.*(-c/b)+6.)-64.)/a;float k=7.*log(2.)*lut_pow2(7.-14.*c/b)/(a*b);if(l>=t)return (log2(a*l+64.)-6.)/14.*b+c;return (l-t)/k;}
 float y=pow(max(l*203./10000.,0.),2610./16384.);return pow((3424./4096.+2413./128.*y)/(1.+2392./128.*y),2523./32.);
}
float lut_log_dec(float v,int s){
 if(s==2){if(v<0.)return -.05641088;if(v<47.28711236*.06641088*.06641088)return sqrt(v/47.28711236)-.05641088;return lut_pow2((v-.69336945)/.08550479)-.00964052;}
 if(s==3||s==4){if(v>=171.2102946929/1023.)return pow(10.,(v*1023.-420.)/261.5)*.19-.01;return (v*1023.-95.)*.01125/76.2102946929;}
 if(s==5){if(v<.181)return (v-.125)/5.6;return pow(10.,(v-.598206)/.241514)-.00873;}
 if(s==6){if(v>.1496578)return (pow(10.,(v-.385537)/.24719)-.052272)/5.555556;return (v-.092809)/5.367655;}
 if(s==7){float a=262128./117.45,b=928./1023.,c=95./1023.;float t=(lut_pow2(14.*(-c/b)+6.)-64.)/a;float k=7.*log(2.)*lut_pow2(7.-14.*c/b)/(a*b);if(v>=0.)return (lut_pow2(14.*(v-c)/b+6.)-64.)/a;return v*k+t;}
 float p=pow(max(v,0.),32./2523.);return 10000.*pow(max(p-3424./4096.,0.)/(2413./128.-2392./128.*p),16384./2610.)/203.;
}
mat3 lut_to_gamut(int s){if(s==2)return mat3(0.627403896,0.069097289,0.016391439,0.329283038,0.919540395,0.088013308,0.043313066,0.011362316,0.895595253); if(s==3)return mat3(0.645679478,0.087529992,0.036957420,0.259114547,0.759699563,0.129280905,0.095205975,0.152770446,0.833761675); if(s==4)return mat3(0.566049080,0.076961379,0.022350913,0.342763087,0.799054500,0.108612051,0.091187833,0.123984121,0.869037036); if(s==5)return mat3(0.585196147,0.078588567,0.022794238,0.322641622,0.819627115,0.114217024,0.092162231,0.101784318,0.862988738); if(s==6)return mat3(0.631321023,0.036819934,0.017369732,0.270800727,0.793036966,0.148789185,0.097878250,0.170143100,0.833841083); if(s==7)return mat3(0.565836540,0.088625637,0.017750040,0.340330999,0.809346842,0.109447621,0.093832461,0.102027521,0.872802339); if(s==8)return mat3(0.627403896,0.069097289,0.016391439,0.329283038,0.919540395,0.088013308,0.043313066,0.011362316,0.895595253); return mat3(1.);}
mat3 lut_from_gamut(int s){if(s==2)return mat3(1.660491002,-0.124550475,-0.018150763,-0.587641139,1.132899897,-0.100578898,-0.072849863,-0.008349423,1.118729661); if(s==3)return mat3(1.626947410,-0.178515527,-0.044436115,-0.540138539,1.417940927,-0.195919966,-0.086808871,-0.239425400,1.240356081); if(s==4)return mat3(1.877915128,-0.176806981,-0.026201126,-0.794168761,1.350999621,-0.148422262,-0.083746367,-0.174192640,1.174623389); if(s==5)return mat3(1.806575884,-0.170090343,-0.025205784,-0.695697274,1.305955216,-0.154468329,-0.110878610,-0.135864873,1.179674113); if(s==6)return mat3(1.617523436,-0.070572741,-0.021101728,-0.537286622,1.334613062,-0.226953875,-0.080236814,-0.264040321,1.248055603); if(s==7)return mat3(1.893123443,-0.205700358,-0.012705743,-0.780881504,1.340257489,-0.152184876,-0.112241939,-0.134557131,1.164890619); if(s==8)return mat3(1.660491002,-0.124550475,-0.018150763,-0.587641139,1.132899897,-0.100578898,-0.072849863,-0.008349423,1.118729661); return mat3(1.);}
vec3 lut_encode(vec3 e,int s){if(s==0)return e;vec3 l=vec3(lut_e_dec(e.r),lut_e_dec(e.g),lut_e_dec(e.b));if(s==1)return l;l=lut_to_gamut(s)*l;return vec3(lut_log_enc(l.r,s),lut_log_enc(l.g,s),lut_log_enc(l.b,s));}
vec3 lut_decode(vec3 v,int s){if(s==0)return v;vec3 l=v;if(s>1)l=lut_from_gamut(s)*vec3(lut_log_dec(v.r,s),lut_log_dec(v.g,s),lut_log_dec(v.b,s));return vec3(lut_e_enc(l.r),lut_e_enc(l.g),lut_e_enc(l.b));}
vec3 lut_texel(int r,int g,int b){int n=int(u_lutCubeSize),c=int(u_lutColumns);int rows=(n+c-1)/c;return texelFetch(u_lut,ivec2(r+n*(g%c),b*rows+g/c),0).rgb;}
vec3 lut_tetrahedral(vec3 p){
 float m=u_lutCubeSize-1.;vec3 x=p*m;ivec3 a=ivec3(min(floor(x),vec3(m-1.)));vec3 f=x-vec3(a);ivec3 b=a+ivec3(1);
 vec3 c000=lut_texel(a.r,a.g,a.b),c111=lut_texel(b.r,b.g,b.b);
 if(f.r>=f.g){
  if(f.g>=f.b){vec3 c100=lut_texel(b.r,a.g,a.b),c110=lut_texel(b.r,b.g,a.b);return c000*(1.-f.r)+c100*(f.r-f.g)+c110*(f.g-f.b)+c111*f.b;}
  if(f.r>=f.b){vec3 c100=lut_texel(b.r,a.g,a.b),c101=lut_texel(b.r,a.g,b.b);return c000*(1.-f.r)+c100*(f.r-f.b)+c101*(f.b-f.g)+c111*f.g;}
  vec3 c001=lut_texel(a.r,a.g,b.b),c101=lut_texel(b.r,a.g,b.b);return c000*(1.-f.b)+c001*(f.b-f.r)+c101*(f.r-f.g)+c111*f.g;
 }
 if(f.r>=f.b){vec3 c010=lut_texel(a.r,b.g,a.b),c110=lut_texel(b.r,b.g,a.b);return c000*(1.-f.g)+c010*(f.g-f.r)+c110*(f.r-f.b)+c111*f.b;}
 if(f.g>=f.b){vec3 c010=lut_texel(a.r,b.g,a.b),c011=lut_texel(a.r,b.g,b.b);return c000*(1.-f.g)+c010*(f.g-f.b)+c011*(f.b-f.r)+c111*f.r;}
 vec3 c001=lut_texel(a.r,a.g,b.b),c011=lut_texel(a.r,b.g,b.b);return c000*(1.-f.b)+c001*(f.b-f.g)+c011*(f.g-f.r)+c111*f.r;
}
float lut_shaper_at(int i,int ch){int w=int(u_lutRes.x);return texelFetch(u_lut,ivec2(i%w,int(u_lutShaperRow)+i/w),0)[ch];}
float lut_shaper(float v,int ch){float m=u_lutShaperSize-1.;float x=v*m;int i=int(min(floor(x),m-1.));return mix(lut_shaper_at(i,ch),lut_shaper_at(i+1,ch),x-float(i));}
vec3 lut_apply(vec3 v){
 vec3 p=clamp((v-u_lutDomainMin)/(u_lutDomainMax-u_lutDomainMin),0.,1.);
 if(u_lutShaperSize>.5){
  p=vec3(lut_shaper(p.r,0),lut_shaper(p.g,1),lut_shaper(p.b,2));
  if(u_lutCubeSize<.5)return p;
  p=clamp((p-u_lutCubeRange.x)/(u_lutCubeRange.y-u_lutCubeRange.x),0.,1.);
 }
 return lut_tetrahedral(p);
}
void main(){
vec4 c=texture(u_tex,v_st);if(u_has_lut<.5||u_amount<=0.||c.a<=0.){o=c;return;}
 vec3 x=c.rgb/c.a;
 vec3 result=lut_decode(lut_apply(lut_encode(x,int(u_inputSpace+.5))),int(u_outputSpace+.5));
 o=vec4(mix(x,result,clamp(u_amount/100.,0.,1.))*c.a,c.a);
}
