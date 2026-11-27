/*
 * Generated agent-style corpus. See README.md in this folder.
 */
import { EFFECTS } from './effects';
import { EXTENSION_LAYERS, SHADER_LAYERS } from './layers';
import { TRANSITIONS } from './transitions';

export type GeneratedShader =
  | { kind: 'effect'; name: string; covers: string[]; prompt: string; def: Record<string, any> }
  | { kind: 'transition'; name: string; covers: string[]; prompt: string; def: Record<string, any> }
  | { kind: 'extension'; name: string; covers: string[]; prompt: string; def: Record<string, any> }
  | { kind: 'shader'; name: string; covers: string[]; prompt: string; code: string };

export const GENERATED: GeneratedShader[] = [...EFFECTS, ...TRANSITIONS, ...SHADER_LAYERS, ...EXTENSION_LAYERS];

/*
 * Section 11 contract rows and the tags that cover them. S3's report script
 * fails the coverage table when a row has no generated case.
 */
export const CONTRACT_ROWS: Record<string, string[]> = {
  'effect varyings and uniforms (v_uv, v_st, v_px, u_res, u_texel, u_time, u_coordRes, u_pxOrigin)': ['coordinates', 'viewportSafe', 'time'],
  'u_pass and 1 to 8 ping-pong passes': ['multipass'],
  'keepOrig (u_orig)': ['keepOrig'],
  'backdrop (u_backdrop on every pass)': ['backdrop'],
  'named params u_<k> and positional aliases u_p<i>, u_c<i>': ['positionalAlias', 'color'],
  'rawShader passthrough': ['rawShader'],
  'viewportSafe: false': ['viewportSafeFalse'],
  'helpers (src, luma, hue2rgb, rgb2hsv, hsv2rgb, hash, noise, fbm)': ['helpers'],
  'effect toggles are float': ['toggle'],
  'transitions (u_from, u_to, u_prog, u_res, u_texel, u_time)': ['transition'],
  'shader layers (iResolution, iTime, iGlobalTime, iProgress, iFrame, iMouse, uv, fragColor, @param)': ['shaderLayer', 'iTime', 'iGlobalTime', 'iProgress', 'iFrame', 'iMouse'],
  'shader-layer binding quirk: vec2, vec4, int @param stay 0': ['intParamStaysZero', 'vec2ParamStaysZero'],
  'extension fragment layers, toggles as uniform bool': ['extensionLayer'],
  'header helpers (SDF, noise, palette)': ['sdf', 'palette'],
  'data-dependent texture loops': ['dataDependentLoop'],
  'dFdy (GL orientation) and dFdx, fwidth': ['dFdy', 'dFdx', 'fwidth'],
  'gl_FragCoord (GL origin)': ['gl_FragCoord'],
};
