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
// A value the controls can't hold (an expression's, or a stored one out of
// range) is held to the control's range, an infinity at its nearer end; one
// that is not a number is neutral. (By bits: fast math folds NaN compares.)
float pm_p(float v,float lo,float hi,float neutral){uint b=floatBitsToUint(v);if((b&0x7f800000u)!=0x7f800000u)return clamp(v,lo,hi);if((b&0x007fffffu)!=0u)return neutral;return (b&0x80000000u)!=0u?lo:hi;}
vec3 pm_p3(float r,float g,float b,float lo,float hi,float neutral){return vec3(pm_p(r,lo,hi,neutral),pm_p(g,lo,hi,neutral),pm_p(b,lo,hi,neutral));}
void main(){
 vec4 c=texture(u_tex,v_st);if(c.a<=0.){o=c;return;}
 float temperature=pm_p(u_temperature,-100.,100.,0.),tint=pm_p(u_tint,-100.,100.,0.),exposure=pm_p(u_exposure,-8.,8.,0.);
 float contrast=pm_p(u_contrast,-100.,100.,0.),pivot=pm_p(u_pivot,0.,1.,.435),saturation=pm_p(u_saturation,0.,200.,100.);
 float liftMaster=pm_p(u_liftMaster,-1.,1.,0.),gammaMaster=pm_p(u_gammaMaster,-2.,2.,0.),gainMaster=pm_p(u_gainMaster,0.,8.,1.),offsetMaster=pm_p(u_offsetMaster,-1.,1.,0.);
 vec3 liftRGB=pm_p3(u_liftRed,u_liftGreen,u_liftBlue,-1.,1.,0.),gammaRGB=pm_p3(u_gammaRed,u_gammaGreen,u_gammaBlue,-2.,2.,0.);
 vec3 gainRGB=pm_p3(u_gainRed,u_gainGreen,u_gainBlue,0.,8.,1.),offsetRGB=pm_p3(u_offsetRed,u_offsetGreen,u_offsetBlue,-1.,1.,0.);
 vec3 x=c.rgb/c.a,y=x;
 // White balance and exposure are gains on light; the balance keeps grey's luminance.
 if(temperature!=0.||tint!=0.||exposure!=0.){
  vec3 g=exp2(vec3(temperature,-tint,-temperature)*.005);
  g*=exp2(exposure)/dot(g,vec3(.2126,.7152,.0722));
  y=vec3(pm_e_enc(pm_e_dec(y.r)*g.r),pm_e_enc(pm_e_dec(y.g)*g.g),pm_e_enc(pm_e_dec(y.b)*g.b));
 }
 if(contrast!=0.)y=(y-pivot)*(1.+contrast/100.)+pivot;
 // Lift, gamma, gain and offset as an ASC CDL: slope, offset, then power.
 vec3 lift=liftMaster+liftRGB;
 vec3 gain=gainMaster*gainRGB;
 vec3 slope=gain*(1.-lift),offset=gain*lift+offsetMaster+offsetRGB;
 vec3 power=exp2(-(gammaMaster+gammaRGB));
 if(slope!=vec3(1.)||offset!=vec3(0.))y=y*slope+offset;
 if(power!=vec3(1.))y=vec3(pm_power(y.r,power.r),pm_power(y.g,power.g),pm_power(y.b,power.b));
 if(saturation!=100.){float l=dot(y,vec3(.2126,.7152,.0722));y=l+(y-l)*(saturation/100.);}
 // A channel the grade leaves alone keeps its stored bits.
 vec3 graded=y*c.a;
 o=vec4(y.r==x.r?c.r:graded.r,y.g==x.g?c.g:graded.g,y.b==x.b?c.b:graded.b,c.a);
}
