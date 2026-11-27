// @ts-nocheck: verbatim reference source (scripts/freeze-reference.ts)
/* Pure math and time helpers of legacy/core/util.ts. */
export function install(PM: any): void {
/* ── math ──────────────────────────────────────────────── */
const clamp = (v: any, a: any, b: any) => (v < a ? a : v > b ? b : v);
const lerp = (a: any, b: any, t: any) => a + (b - a) * t;
const round = (v: any, n = 2) => { const p = 10 ** n; return Math.round(v * p) / p; };
PM.clamp = clamp; PM.lerp = lerp; PM.round = round;
/* Colours are #RGB, #RRGGBB, or with alpha #RGBA / #RRGGBBAA. Channels are
   read by position so eight digits never overflow into the wrong channel. */
const hexDigits = (hex: any): string => {
  const s = String(hex ?? '').replace('#', '');
  return s.length === 3 || s.length === 4 ? s.split('').map(c => c + c).join('') : s;
};
PM.hex2rgb = (hex: any) => {
  const s = hexDigits(hex);
  return [0, 2, 4].map(i => (parseInt(s.slice(i, i + 2), 16) || 0) / 255);
};
PM.hexAlpha = (hex: any) => {
  const s = hexDigits(hex);
  return s.length === 8 ? (parseInt(s.slice(6, 8), 16) || 0) / 255 : 1;
};
PM.rgb2hex = (r: any, g: any, b: any) => '#' + [r, g, b].map(v => clamp(Math.round(v * 255), 0, 255).toString(16).padStart(2, '0')).join('');

/* ── time ──────────────────────────────────────────────── */
PM.tc = (sec: any, fps = 30, showFrames = true) => {
  const neg = sec < 0; sec = Math.abs(sec);
  let f = Math.round(sec * fps);
  const ff = f % fps; f = Math.floor(f / fps);
  const ss = f % 60; const mm = Math.floor(f / 60) % 60; const hh = Math.floor(f / 3600);
  const p = (n: any, w = 2) => String(n).padStart(w, '0');
  const base = (hh ? p(hh) + ':' : '') + p(mm) + ':' + p(ss);
  return (neg ? '-' : '') + base + (showFrames ? ':' + p(ff) : '');
};
PM.parseTc = (str: any, fps = 30) => {
  const parts: any = String(str).trim().split(':').map(Number);
  if (parts.some(isNaN)) return null;
  let s = 0;
  if (parts.length === 4) s = parts[0] * 3600 + parts[1] * 60 + parts[2] + parts[3] / fps;
  else if (parts.length === 3) s = parts[0] * 60 + parts[1] + parts[2] / fps;
  else if (parts.length === 2) s = parts[0] + parts[1] / fps;
  else s = parts[0];
  return s;
};
PM.snapF = (t: any, fps: any) => Math.round(t * fps) / fps;

}
