#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform float u_temperature, u_tint, u_exposure, u_contrast, u_pivot, u_saturation;
uniform float u_liftMaster, u_liftRed, u_liftGreen, u_liftBlue;
uniform float u_gammaMaster, u_gammaRed, u_gammaGreen, u_gammaBlue;
uniform float u_gainMaster, u_gainRed, u_gainGreen, u_gainBlue;
uniform float u_offsetMaster, u_offsetRed, u_offsetGreen, u_offsetBlue;
out vec4 o;
float pm_e_dec(float v){float m=abs(v);float l=m<=.04045?m/12.92:pow((m+.055)/1.055,2.4);return v<0.?-l:l;}
float pm_e_enc(float l){float m=abs(l);float e=m<=.0031308?12.92*m:1.055*pow(m,1./2.4)-.055;return l<0.?-e:e;}
float pm_power(float v,float p){return v>0.?pow(v,p):v;}
void main(){
 vec4 c=texture(u_tex,v_st);if(c.a<=0.){o=c;return;}
 vec3 x=c.rgb/c.a,y=x;
 // White balance and exposure are gains on light; the balance keeps grey's luminance.
 if(u_temperature!=0.||u_tint!=0.||u_exposure!=0.){
  vec3 g=exp2(vec3(u_temperature,-u_tint,-u_temperature)*.005);
  g*=exp2(u_exposure)/dot(g,vec3(.2126,.7152,.0722));
  y=vec3(pm_e_enc(pm_e_dec(y.r)*g.r),pm_e_enc(pm_e_dec(y.g)*g.g),pm_e_enc(pm_e_dec(y.b)*g.b));
 }
 if(u_contrast!=0.)y=(y-u_pivot)*(1.+u_contrast/100.)+u_pivot;
 // Lift, gamma, gain and offset as an ASC CDL: slope, offset, then power.
 vec3 lift=u_liftMaster+vec3(u_liftRed,u_liftGreen,u_liftBlue);
 vec3 gain=u_gainMaster*vec3(u_gainRed,u_gainGreen,u_gainBlue);
 vec3 slope=gain*(1.-lift),offset=gain*lift+u_offsetMaster+vec3(u_offsetRed,u_offsetGreen,u_offsetBlue);
 vec3 power=exp2(-(u_gammaMaster+vec3(u_gammaRed,u_gammaGreen,u_gammaBlue)));
 if(slope!=vec3(1.)||offset!=vec3(0.))y=y*slope+offset;
 if(power!=vec3(1.))y=vec3(pm_power(y.r,power.r),pm_power(y.g,power.g),pm_power(y.b,power.b));
 if(u_saturation!=100.){float l=dot(y,vec3(.2126,.7152,.0722));y=l+(y-l)*(u_saturation/100.);}
 // A channel the grade leaves alone keeps its stored bits.
 vec3 graded=y*c.a;
 o=vec4(y.r==x.r?c.r:graded.r,y.g==x.g?c.g:graded.g,y.b==x.b?c.b:graded.b,c.a);
}
