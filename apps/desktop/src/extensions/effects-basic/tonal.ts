import type { EffectDefinition, EffectParamDefinition } from 'powermove';
import { HSL, imageFragment } from './core-image';

const number = (k: string, label: string, def: number, min: number, max: number, unit = ''): EffectParamDefinition =>
  ({ k, label, def, min, max, unit, step: .1, advanced: false });
const color = (k: string, label: string, def: string): EffectParamDefinition => ({ k, label, def, type: 'color', advanced: false });
const toggle = (k: string, label: string): EffectParamDefinition => ({ k, label, def: false, type: 'toggle', advanced: false });
const effect = (id: string, label: string, params: EffectParamDefinition[], body: string, helpers = ''): EffectDefinition =>
  ({ id, label, group: 'Color', params, passes: 1, rawShader: true, viewportSafe: true, viewportPadding: [], frag: imageFragment(params, body, helpers) });

const tritone = effect('tritone', 'Tritone', [
  color('highlights', 'Highlights', '#FFFFFF'), color('midtones', 'Midtones', '#7F5F3F'), color('shadows', 'Shadows', '#000000'),
  number('blendWithOriginal', 'Blend With Original', 0, 0, 100, '%'),
], `
  float luminance=luma(rgb);
  vec3 mapped=luminance<.5 ? mix(u_shadows,u_midtones,2.*luminance) : mix(u_midtones,u_highlights,2.*luminance-1.);
  o=vec4(mix(mapped,rgb,u_blendWithOriginal/100.)*c.a,c.a);
`);

const balanceHLS = effect('colorBalanceHLS', 'Color Balance (HLS)', [
  number('hue', 'Hue', 0, -360, 360, '°'), number('lightness', 'Lightness', 0, -100, 100, '%'),
  number('saturation', 'Saturation', 0, -100, 100, '%'),
], `
  if(u_hue==0.&&u_lightness==0.&&u_saturation==0.){o=c;return;}
  vec3 hls=rgbToHsl(clamp(rgb,0.,1.));
  hls.x=fract(hls.x+u_hue/360.);
  hls.y=u_lightness>=0. ? mix(hls.y,1.,u_lightness/100.) : hls.y*(1.+u_lightness/100.);
  hls.z=clamp(hls.z*(1.+u_saturation/100.),0.,1.);
  o=vec4(hslToRgb(hls)*c.a,c.a);
`, HSL);

const brightness = effect('brightnessContrast', 'Brightness & Contrast', [
  number('brightness', 'Brightness', 0, -150, 150), number('contrast', 'Contrast', 0, -100, 100), toggle('useLegacy', 'Use Legacy'),
], `
  if(u_brightness==0.&&u_contrast==0.){o=c;return;}
  vec3 mapped;
  if(u_useLegacy>.5) mapped=(rgb-.5)*(1.+u_contrast/100.)+.5+u_brightness/100.;
  else {
    float amount=u_brightness/150.;
    mapped=amount>=0. ? mix(rgb,vec3(1.),amount) : rgb*(1.+amount);
    float slope=u_contrast>=0. ? 1./max(.01,1.-u_contrast/100.) : 1.+u_contrast/100.;
    mapped=(mapped-.5)*slope+.5;
  }
  o=vec4(mapped*c.a,c.a);
`);

const weights = ['Reds','Yellows','Greens','Cyans','Blues','Magentas'];
const defaults = [40,60,40,60,20,80];
const monochromeParams: EffectParamDefinition[] = weights.map((label,index)=>({
  ...number(label.toLowerCase(),label,defaults[index]!, -200,300,'%'),group:'Color Weights',
}));
monochromeParams.push(toggle('tint','Tint'),color('tintColor','Tint Color','#D7B68C'));
const monochrome = effect('blackWhite', 'Black & White', monochromeParams, `
  vec3 bounded=clamp(rgb,0.,1.);
  vec3 hls=rgbToHsl(bounded);
  float hue=fract(hls.x)*6.; int sector=int(floor(hue)); float fraction=fract(hue);
  float a=u_reds,b=u_yellows;
  if(sector==1){a=u_yellows;b=u_greens;}
  else if(sector==2){a=u_greens;b=u_cyans;}
  else if(sector==3){a=u_cyans;b=u_blues;}
  else if(sector==4){a=u_blues;b=u_magentas;}
  else if(sector==5){a=u_magentas;b=u_reds;}
  float low=min(bounded.r,min(bounded.g,bounded.b)),high=max(bounded.r,max(bounded.g,bounded.b));
  float gray=low+(high-low)*mix(a,b,fraction)/100.;
  vec3 mapped=vec3(gray);
  if(u_tint>.5){vec3 tintHLS=rgbToHsl(clamp(u_tintColor,0.,1.));mapped=hslToRgb(vec3(tintHLS.x,clamp(gray,0.,1.),tintHLS.z));}
  o=vec4(mapped*c.a,c.a);
`,HSL);

export const TONAL_EFFECTS: EffectDefinition[] = [tritone,balanceHLS,brightness,monochrome];
