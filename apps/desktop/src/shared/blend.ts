export const BLEND_NAMES = [
  'normal',
  'add',
  'screen',
  'multiply',
  'overlay',
  'softlight',
  'difference',
  'lighten',
  'darken',
  'colorburn',
  'colordodge',
  'hardlight',
  'exclusion',
  'hue',
  'saturation',
  'color',
  'luminosity',
  'linearburn',
] as const;
export const BLEND_GLSL = `
float luminosity(vec3 c){return dot(c,vec3(.3,.59,.11));}
float saturation(vec3 c){return max(c.r,max(c.g,c.b))-min(c.r,min(c.g,c.b));}
vec3 clipColor(vec3 c){float l=luminosity(c),n=min(c.r,min(c.g,c.b)),x=max(c.r,max(c.g,c.b));if(n<0.)c=l+(c-l)*l/max(l-n,1e-8);if(x>1.)c=l+(c-l)*(1.-l)/max(x-l,1e-8);return c;}
vec3 setLum(vec3 c,float l){return clipColor(c+vec3(l-luminosity(c)));}
vec3 setSat(vec3 c,float s){float n=min(c.r,min(c.g,c.b)),x=max(c.r,max(c.g,c.b));return x>n?(c-n)*s/(x-n):vec3(0.);}
vec3 blend(vec3 b, vec3 s, int m){
  if(m==1) return b+s;
  if(m==2) return 1.-(1.-b)*(1.-s);
  if(m==3) return b*s;
  if(m==4) return mix(2.*b*s, 1.-2.*(1.-b)*(1.-s), step(.5,b));
  if(m==5) return mix(2.*b*s+b*b*(1.-2.*s), sqrt(b)*(2.*s-1.)+2.*b*(1.-s), step(.5,s));
  if(m==6) return abs(b-s);
  if(m==7) return max(b,s);
  if(m==8) return min(b,s);
  if(m==9) return mix(1.-min(vec3(1.),(1.-b)/max(s,vec3(1e-8))),vec3(1.),step(vec3(1.),b));
  if(m==10) return mix(min(vec3(1.),b/max(1.-s,vec3(1e-8))),vec3(0.),step(b,vec3(0.)));
  if(m==11) return mix(2.*b*s,1.-2.*(1.-b)*(1.-s),step(.5,s));
  if(m==12) return b+s-2.*b*s;
  if(m==13) return setLum(setSat(s,saturation(b)),luminosity(b));
  if(m==14) return setLum(setSat(b,saturation(s)),luminosity(b));
  if(m==15) return setLum(s,luminosity(b));
  if(m==16) return setLum(b,luminosity(s));
  if(m==17) return max(vec3(0.),b+s-1.);
  if(m==18){vec3 D=mix(((16.*b-12.)*b+4.)*b,sqrt(b),step(.25,b));return mix(b-(1.-2.*s)*b*(1.-b),b+(2.*s-1.)*(D-b),step(.5,s));}
  return s;
}
`;
