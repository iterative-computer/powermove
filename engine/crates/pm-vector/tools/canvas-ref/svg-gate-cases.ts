/**
 * Chromium references for every source construct the level 1 SVG gate
 * admits (docs/engine/vector.md, "What level 1 draws"). Each document
 * exercises one construct with paint the replay can draw in one kind of
 * pass (analytic shapes only, or tessellated paths only), so a reference
 * measures the construct rather than the multisampled-pass mix.
 *
 * `run.ts --svg-gate` captures them into `svg-gate-<stamp>-<commit>/` and
 * writes `tests/fixtures/svg-gate-cases.json`; `tests/svg.rs` pins the gate
 * verdict of every case and requires every admitted case to be inside the
 * 2D class against the capture. A construct the gate admits must have a
 * document here.
 */
import type { SvgCase } from './cases';

const NS = 'xmlns="http://www.w3.org/2000/svg"';
const svg = (attrs: string, body: string) => `<svg ${NS}${attrs ? ' ' + attrs : ''}>${body}</svg>`;

/** Analytic paint over a 100 × 60 user space: a plate, a rect, a round rect, a circle, an ellipse. */
const ANALYTIC_100x60 =
  '<rect width="100" height="60" fill="#1B2A4A"/>' +
  '<rect x="8.5" y="7.25" width="30" height="20" fill="#3FCF8E"/>' +
  '<rect x="48" y="6" width="44" height="22" rx="6" fill="#F2C14E"/>' +
  '<circle cx="24" cy="44" r="11" fill="#FF6B1A"/>' +
  '<ellipse cx="70" cy="44" rx="20" ry="9" fill="#4C8DFF"/>';
/** The same over a viewBox of any size (`w × h`). */
const analytic = (x: number, y: number, w: number, h: number) =>
  `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="#1B2A4A"/>` +
  `<rect x="${x + w * 0.08}" y="${y + h * 0.1}" width="${w * 0.3}" height="${h * 0.35}" fill="#3FCF8E"/>` +
  `<circle cx="${x + w * 0.25}" cy="${y + h * 0.72}" r="${Math.min(w, h) * 0.18}" fill="#FF6B1A"/>` +
  `<ellipse cx="${x + w * 0.7}" cy="${y + h * 0.5}" rx="${w * 0.2}" ry="${h * 0.3}" fill="#4C8DFF"/>`;
/** Tessellated paint over a 100 × 60 user space. */
const TESSELLATED_100x60 =
  '<path d="M6 54 L30 6 L54 54 Z" fill="#3FCF8E"/>' +
  '<polygon points="58,8 94,14 88,52 62,46" fill="#FF6B1A"/>';

const ANALYTIC_SIZES: Array<[number, number]> = [[128, 128], [200, 120], [97, 61]];

type Doc = [id: string, svg: string, sizes?: Array<[number, number]>];

function sizing(): Doc[] {
  const docs: Doc[] = [
    ['size-vb', svg('viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-equal', svg('width="100" height="60" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-double', svg('width="200" height="120" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-fraction', svg('width="99.5" height="60.25" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-px', svg('width="150px" height="90px" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-percent', svg('width="100%" height="100%" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-auto', svg('width="auto" height="auto" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-w-only', svg('width="150" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-h-only', svg('height="90" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-wh-no-vb', svg('width="100" height="60"', ANALYTIC_100x60)],
    ['size-w-only-no-vb', svg('width="100"', ANALYTIC_100x60)],
    ['size-no-size', svg('', ANALYTIC_100x60)],
    ['size-vb-origin', svg('viewBox="10 5 100 60"', analytic(10, 5, 100, 60))],
    ['size-vb-negative', svg('viewBox="-20 -10 100 60"', analytic(-20, -10, 100, 60))],
    ['size-vb-commas', svg('viewBox="0,0,100,60"', ANALYTIC_100x60)],
    ['size-wh-vb-origin', svg('width="200" height="120" viewBox="10 5 100 60"', analytic(10, 5, 100, 60))],
    ['size-root-xy', svg('x="10" y="7" width="100" height="60" viewBox="0 0 100 60"', ANALYTIC_100x60)],
    ['size-icon', svg('width="96" height="96" viewBox="0 0 24 24"', '<path d="M12 2 L22 20 L2 20 Z" fill="#3FCF8E"/>'), [[96, 96], [48, 48], [131, 131]]],
    ['size-icon-tess', svg('width="200" height="120" viewBox="0 0 100 60"', TESSELLATED_100x60)],
  ];
  // preserveAspectRatio with a viewBox whose aspect differs from the
  // viewport's, wide and tall.
  const aligns = ['xMinYMin', 'xMidYMin', 'xMaxYMin', 'xMinYMid', 'xMidYMid', 'xMaxYMid', 'xMinYMax', 'xMidYMax', 'xMaxYMax'];
  const values = ['none', ...aligns.flatMap(a => [a, `${a} slice`])];
  values.push('xMidYMid meet');
  for (const v of values) {
    const tag = v.replace(' ', '-');
    docs.push([`par-wide-${tag}`, svg(`width="100" height="100" viewBox="0 0 160 60" preserveAspectRatio="${v}"`, analytic(0, 0, 160, 60)), [[100, 100], [160, 160]]]);
    docs.push([`par-tall-${tag}`, svg(`width="100" height="100" viewBox="0 0 60 160" preserveAspectRatio="${v}"`, analytic(0, 0, 60, 160)), [[100, 100], [160, 160]]]);
  }
  // A viewBox-only document is laid out in the bucket: the bucket's aspect decides.
  docs.push(['par-vb-only-slice', svg('viewBox="0 0 160 60" preserveAspectRatio="xMinYMax slice"', analytic(0, 0, 160, 60))]);
  docs.push(['par-vb-only-none', svg('viewBox="0 0 160 60" preserveAspectRatio="none"', analytic(0, 0, 160, 60))]);
  return docs;
}

function transforms(): Doc[] {
  const list: Array<[string, string]> = [
    ['translate', 'translate(13.5 7.25)'],
    ['translate1', 'translate(9)'],
    ['rotate', 'rotate(-30 50 30)'],
    ['rotate-commas', 'rotate(25,50,30)'],
    ['scale', 'scale(1.3 0.8)'],
    ['scale1', 'scale(0.75)'],
    ['skewx', 'skewX(20)'],
    ['skewy', 'skewY(-15)'],
    ['matrix', 'matrix(0.9 0.3 -0.2 1.1 8 -4)'],
    ['list', 'translate(50 30) rotate(15) scale(0.8) translate(-50 -30)'],
  ];
  const docs: Doc[] = [];
  for (const [id, t] of list) {
    docs.push([`xform-analytic-${id}`, svg('viewBox="0 0 100 60"',
      `<rect x="8.5" y="7.25" width="30" height="20" fill="#3FCF8E" transform="${t}"/>` +
      `<rect x="48" y="6" width="40" height="20" rx="6" fill="#F2C14E" transform="${t}"/>` +
      `<circle cx="24" cy="44" r="11" fill="#FF6B1A" transform="${t}"/>` +
      `<ellipse cx="70" cy="44" rx="18" ry="8" fill="#4C8DFF" transform="${t}"/>`)]);
    docs.push([`xform-tess-${id}`, svg('viewBox="0 0 100 60"',
      `<path d="M6 54 L30 6 L54 54 Z" fill="#3FCF8E" transform="${t}"/>` +
      `<polygon points="58,8 94,14 88,52 62,46" fill="#FF6B1A" transform="${t}"/>` +
      `<polyline points="40,10 50,30 60,12" fill="#A73366" transform="${t}"/>`)]);
    docs.push([`xform-group-${id}`, svg('viewBox="0 0 100 60"', `<g transform="${t}">${ANALYTIC_100x60.replace('<rect width="100" height="60" fill="#1B2A4A"/>', '')}</g>`)]);
  }
  // A shape transform with everything a shape's own property nodes carry.
  docs.push(['xform-shape-opacity', svg('viewBox="0 0 100 60"', '<rect width="100" height="60" fill="#1B2A4A"/><rect x="10" y="10" width="50" height="30" rx="5" fill="#F2C14E" opacity="0.6" transform="rotate(-12 35 25)"/><circle cx="60" cy="35" r="18" fill="#4C8DFF" opacity="0.5" transform="scale(1.1 0.9)"/>')]);
  docs.push(['xform-shape-clip', svg('viewBox="0 0 100 60"', '<defs><clipPath id="c"><rect x="10" y="5" width="40" height="30" rx="8"/></clipPath></defs><rect x="0" y="0" width="60" height="40" fill="#3FCF8E" clip-path="url(#c)" transform="translate(20 10) rotate(10)"/>')]);
  docs.push(['xform-shape-gradient', svg('viewBox="0 0 100 60"', '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#FF6B1A"/><stop offset="1" stop-color="#4C8DFF"/></linearGradient></defs><rect x="10" y="10" width="60" height="30" rx="6" fill="url(#g)" transform="rotate(15 40 25) translate(5 3)"/><ellipse cx="75" cy="40" rx="15" ry="10" fill="url(#g)" transform="skewX(-10)"/>')]);
  docs.push(['xform-shape-gradient-tess', svg('viewBox="0 0 100 60"', '<defs><radialGradient id="r"><stop offset="0" stop-color="#fff"/><stop offset="0.7" stop-color="#3FCF8E"/><stop offset="1" stop-color="#1B2A4A"/></radialGradient></defs><path d="M10 50 L40 5 L70 50 Z" fill="url(#r)" transform="translate(10 2) scale(1.1)"/>')]);
  docs.push(['xform-nested-shape', svg('viewBox="0 0 100 60"', '<g transform="translate(10 5)"><g transform="scale(0.9)"><rect x="10" y="10" width="40" height="25" fill="#3FCF8E" transform="rotate(20 30 22)"/><circle cx="70" cy="30" r="12" fill="#FF6B1A" transform="translate(-5 4)"/></g></g>')]);
  docs.push(['xform-group-clip', svg('viewBox="0 0 100 60"', '<defs><clipPath id="c"><circle cx="40" cy="30" r="22"/></clipPath></defs><g transform="translate(10 0)" clip-path="url(#c)"><rect width="100" height="30" fill="#3FCF8E"/><rect y="30" width="100" height="30" fill="#1B2A4A"/></g>')]);
  docs.push(['xform-clip-rotated', svg('viewBox="0 0 100 60"', '<defs><clipPath id="c" transform="rotate(20 50 30)"><rect x="20" y="15" width="60" height="30"/></clipPath></defs><g clip-path="url(#c)"><rect width="100" height="60" fill="#F2C14E"/><circle cx="50" cy="30" r="20" fill="#A73366"/></g>')]);
  docs.push(['xform-single-top', svg('viewBox="0 0 100 60"', '<rect x="20" y="10" width="40" height="30" fill="#3FCF8E" transform="rotate(10 40 25)"/>')]);
  docs.push(['xform-single-top-sized', svg('width="200" height="120" viewBox="0 0 100 60"', '<path d="M20 10 L60 15 L40 50 Z" fill="#4C8DFF" transform="translate(15 5)"/>')]);
  docs.push(['xform-identity', svg('viewBox="0 0 100 60"', '<rect x="20" y="10" width="40" height="30" fill="#3FCF8E" transform="translate(0 0)"/><circle cx="70" cy="30" r="15" fill="#FF6B1A" transform="rotate(360 70 30)"/>')]);
  docs.push(['xform-root-sized', svg('width="150" height="90" viewBox="5 5 100 60"', '<g transform="rotate(-8 50 30)"><rect x="15" y="15" width="40" height="30" fill="#3FCF8E"/></g><ellipse cx="75" cy="40" rx="15" ry="10" fill="#FF6B1A" transform="translate(-4 2)"/>')]);
  return docs;
}

function gradients(): Doc[] {
  const stops2 = '<stop offset="0" stop-color="#FF6B1A"/><stop offset="1" stop-color="#4C8DFF"/>';
  const docs: Doc[] = [
    ['grad-percent', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="g" x1="10%" y1="0%" x2="90%" y2="100%">${stops2}</linearGradient></defs><rect x="5" y="5" width="90" height="50" rx="8" fill="url(#g)"/>`)],
    ['grad-user-percent', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0%" y1="0%" x2="100%" y2="0%">${stops2}</linearGradient></defs><rect x="20" y="5" width="60" height="50" fill="url(#g)"/>`)],
    ['grad-href', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="base">${stops2}</linearGradient><linearGradient id="g" href="#base" x2="0" y2="1"/><linearGradient id="g2" xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="#g" y1="0.2"/></defs><rect x="5" y="5" width="40" height="50" fill="url(#g)"/><circle cx="72" cy="30" r="22" fill="url(#g2)"/>`)],
    ['grad-stops', svg('viewBox="0 0 100 60"', '<defs><linearGradient id="g"><stop offset="20%" style="stop-color:#1B2A4A"/><stop offset="0.1" stop-color="#3FCF8E"/><stop offset="80%" stop-color="#F2C14E" stop-opacity="0.5"/><stop offset="1.5" stop-color="#A73366"/></linearGradient></defs><rect x="5" y="5" width="90" height="50" fill="url(#g)"/>')],
    ['grad-pad', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="g" spreadMethod="pad" x1="0.3" x2="0.7">${stops2}</linearGradient></defs><rect x="5" y="5" width="90" height="50" fill="url(#g)"/>`)],
    ['grad-radial-transform', svg('viewBox="0 0 100 60"', '<defs><radialGradient id="r" gradientTransform="translate(0.5 0.5) scale(1 0.5) rotate(30) translate(-0.5 -0.5)"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#4C8DFF"/></radialGradient></defs><rect x="5" y="5" width="90" height="50" rx="10" fill="url(#r)"/>')],
    ['grad-radial-percent', svg('viewBox="0 0 100 60"', '<defs><radialGradient id="r" cx="40%" cy="60%" r="70%" fx="40%" fy="60%"><stop offset="0" stop-color="#F2C14E"/><stop offset="1" stop-color="#A73366"/></radialGradient></defs><ellipse cx="50" cy="30" rx="45" ry="25" fill="url(#r)"/>')],
    ['grad-radial-user', svg('viewBox="0 0 100 60"', '<defs><radialGradient id="r" gradientUnits="userSpaceOnUse" cx="50" cy="30" r="40"><stop offset="0" stop-color="#3FCF8E"/><stop offset="1" stop-color="#1B2A4A" stop-opacity="0.2"/></radialGradient></defs><rect width="100" height="60" fill="url(#r)"/>')],
    ['grad-opacity', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="g">${stops2}</linearGradient></defs><rect width="100" height="60" fill="#E8E2CF"/><g opacity="0.7"><rect x="5" y="5" width="60" height="40" fill="url(#g)" fill-opacity="0.6"/></g>`)],
    ['grad-tess', svg('viewBox="0 0 100 60"', `<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="100" y2="60">${stops2}</linearGradient></defs><path d="M5 55 C 20 0, 60 0, 95 55 Z" fill="url(#g)"/>`)],
    ['grad-sized-root', svg('width="200" height="120" viewBox="0 0 100 60"', `<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="10%" y1="0" x2="90%" y2="0">${stops2}</linearGradient></defs><rect width="100" height="60" fill="url(#g)"/>`)],
  ];
  return docs;
}

function clips(): Doc[] {
  const plate = '<rect width="100" height="60" fill="#3FCF8E"/><circle cx="50" cy="30" r="22" fill="#1B2A4A"/>';
  return [
    ['clip-ellipse', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><ellipse cx="50" cy="30" rx="40" ry="20"/></clipPath></defs><g clip-path="url(#c)">${plate}</g>`)],
    ['clip-rrect', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><rect x="10" y="8" width="80" height="44" rx="12"/></clipPath></defs><g clip-path="url(#c)">${plate}</g>`)],
    ['clip-rect-fraction', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><rect x="10.3" y="8.6" width="70.2" height="40.7"/></clipPath></defs><g clip-path="url(#c)">${plate}</g>`)],
    ['clip-polygon', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><polygon points="50,2 98,58 2,58"/></clipPath></defs><g clip-path="url(#c)">${plate}</g>`)],
    ['clip-rule', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><path clip-rule="evenodd" d="M5 5 H95 V55 H5 Z M30 15 H70 V45 H30 Z"/></clipPath></defs><g clip-path="url(#c)">${plate}</g>`)],
    ['clip-shape', svg('viewBox="0 0 100 60"', '<defs><clipPath id="c"><circle cx="50" cy="30" r="25"/></clipPath></defs><rect width="100" height="60" fill="#F2C14E" clip-path="url(#c)"/>')],
    ['clip-nested-groups', svg('viewBox="0 0 100 60"', `<defs><clipPath id="c"><rect x="10" y="5" width="80" height="50" rx="10"/></clipPath></defs><g clip-path="url(#c)"><g opacity="0.8">${plate}</g></g>`)],
  ];
}

function misc(): Doc[] {
  const v = 'viewBox="0 0 100 60"';
  return [
    ['paint-colors', svg(v,
      '<rect x="2" y="2" width="14" height="25" fill="tomato"/><rect x="18" y="2" width="14" height="25" fill="rgb(20, 120, 220)"/>' +
      '<rect x="34" y="2" width="14" height="25" fill="rgba(20,120,220,0.5)"/><rect x="50" y="2" width="14" height="25" fill="rgb(10%, 60%, 30%)"/>' +
      '<rect x="66" y="2" width="14" height="25" fill="#4C8"/><rect x="82" y="2" width="14" height="25" fill="#4C8DFF80"/>' +
      '<rect x="2" y="32" width="14" height="25" fill="#4C88"/><rect x="18" y="32" width="14" height="25" fill="hsl(200, 60%, 50%)"/>' +
      '<rect x="34" y="32" width="14" height="25" fill="transparent"/><rect x="50" y="32" width="14" height="25" fill="RED"/>' +
      '<rect x="66" y="32" width="14" height="25" fill="  #A73366  "/><rect x="82" y="32" width="14" height="25"/>')],
    ['paint-invalid', svg(v, '<rect x="5" y="5" width="40" height="40" fill="notacolor"/><rect x="55" y="5" width="40" height="40" fill="#12345"/>')],
    ['paint-current-color', svg(`${v} color="#E85D75"`, '<g color="#3FCF8E"><rect x="5" y="5" width="40" height="40" fill="currentColor"/></g><circle cx="72" cy="30" r="20" fill="currentColor"/>')],
    ['paint-none-and-opacity', svg(v, '<rect width="100" height="60" fill="#1B2A4A"/><rect x="5" y="5" width="40" height="40" fill="none"/><rect x="10" y="10" width="40" height="40" fill="#FF6B1A" fill-opacity="0"/><circle cx="60" cy="30" r="20" fill="#3FCF8E" opacity="0"/><ellipse cx="70" cy="30" rx="25" ry="12" fill="#F2C14E" fill-opacity="0.35"/>')],
    ['paint-style', svg(v, '<rect width="100" height="60" style="fill: #1B2A4A"/><rect x="5" y="5" width="40" height="40" style=" fill:#FF6B1A ; fill-opacity: 0.7 ;"/><circle cx="65" cy="30" r="20" style="fill:#3FCF8E;opacity:0.5"/>')],
    ['paint-display', svg(v, '<rect width="100" height="60" fill="#1B2A4A"/><g display="none"><rect width="50" height="50" fill="red"/></g><circle cx="50" cy="30" r="20" fill="#3FCF8E" style="display:none"/><rect x="60" y="10" width="30" height="30" fill="#F2C14E" display="inline"/>')],
    ['paint-shape-rendering', svg(v, '<rect x="5.5" y="5.5" width="40" height="40" fill="#3FCF8E" shape-rendering="geometricPrecision"/><circle cx="70" cy="30" r="20" fill="#FF6B1A" shape-rendering="auto"/>')],
    ['shape-rect-radii', svg(v, '<rect x="2" y="2" width="30" height="25" rx="6" fill="#3FCF8E"/><rect x="35" y="2" width="30" height="25" ry="8" fill="#FF6B1A"/><rect x="68" y="2" width="30" height="25" rx="40" fill="#4C8DFF"/><rect x="2" y="32" width="30" height="25" rx="4" ry="10" fill="#F2C14E"/><rect x="35" y="32" width="30" height="25" rx="auto" ry="5" fill="#A73366"/><rect x="68" y="32" width="30" height="25" rx="-3" ry="6" fill="#E8E2CF"/>')],
    ['shape-degenerate', svg(v, '<rect width="100" height="60" fill="#1B2A4A"/><rect x="5" y="5" width="0" height="40" fill="red"/><rect x="5" y="5" width="-10" height="40" fill="red"/><circle cx="50" cy="30" r="0" fill="red"/><ellipse cx="50" cy="30" rx="10" ry="0" fill="red"/><line x1="0" y1="0" x2="100" y2="60" fill="red"/><path d="M10 10" fill="red"/>')],
    ['shape-ellipse-one-radius', svg(v, '<ellipse cx="30" cy="30" rx="20" fill="#3FCF8E"/><ellipse cx="70" cy="30" ry="15" fill="#FF6B1A"/>')],
    ['path-commands', svg(v, '<path d="m5 5 h30 v20 h-30 z M45 5 l20 0 l0 20 l-20 0 Z M75 5 H95 V25 H75 Z" fill="#3FCF8E"/><path d="M5 35 C 15 30 25 60 35 50 S 45 35 50 45 Q 60 60 70 45 T 95 45 L95 58 L5 58 Z" fill="#FF6B1A"/>')],
    ['path-numbers', svg(v, '<path d="M5,5L45,5,45,25,5,25z M50 5 l.5.5 40 0 0 20-40 0z M5 30 L1e1 55 L4.5e1 55 L45 30 Z" fill="#4C8DFF"/>')],
    ['path-unclosed', svg(v, '<path d="M5 5 L45 10 L25 50" fill="#3FCF8E"/><polyline points="55,5 95,10 75,50 60,30" fill="#FF6B1A"/>')],
    ['path-nonzero-overlap', svg(v, '<path d="M5 5 H60 V50 H5 Z M30 10 H95 V55 H30 Z" fill="#A73366"/><path d="M5 30 H95 V40 H5 Z M50 2 V58 H55 V2 Z" fill="#F2C14E" fill-rule="nonzero"/>')],
    ['path-evenodd-style', svg(v, '<path style="fill-rule:evenodd" d="M5 5 H95 V55 H5 Z M25 15 H75 V45 H25 Z" fill="#3FCF8E"/>')],
    ['points-separators', svg(v, '<polygon points="5,5 45,5 45,55 5,55," fill="#3FCF8E"/><polygon points=" 55 5,95 5 95 55   55,55 70" fill="#FF6B1A"/>')],
    ['paint-inert-stroke', svg(v, '<rect width="100" height="60" fill="#1B2A4A"/><rect x="5.5" y="5.5" width="40" height="40" fill="#3FCF8E" stroke="none" stroke-width="3" stroke-opacity="0.5" stroke-miterlimit="10" stroke-linecap="round" stroke-linejoin="bevel" stroke-dasharray="4 2" stroke-dashoffset="1"/><circle cx="70" cy="30" r="20" fill="#FF6B1A" style="stroke:none;stroke-miterlimit:10;stroke-width:2"/>')],
    ['doc-version', svg(`${v} version="1.1" baseProfile="full"`, '<rect x="10" y="10" width="80" height="40" rx="6" fill="#4C8DFF"/>')],
    ['grad-explicit-defaults', svg(`${v} color="#E85D75"`, '<defs><linearGradient id="g" gradientUnits="objectBoundingBox" spreadMethod="pad" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="currentColor"/><stop offset="1" stop-color="#4C8DFF"/></linearGradient><radialGradient id="r" cx="0.5" cy="0.5" r="0.5" fx="0.5" fy="0.5" fr="0"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#3FCF8E"/></radialGradient></defs><rect x="5" y="5" width="50" height="50" fill="url(#g)"/><circle cx="75" cy="30" r="20" fill="url(#r)"/>')],
    ['clip-explicit', svg(v, '<defs><clipPath id="c" clipPathUnits="userSpaceOnUse"><rect x="10" y="8" width="80" height="44" rx="12"/></clipPath></defs><g clip-path="url(#c)"><rect width="100" height="60" fill="#3FCF8E"/></g><g clip-path="none"><circle cx="50" cy="30" r="10" fill="#1B2A4A"/></g>')],
    ['doc-metadata', `<svg ${NS} xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" viewBox="0 0 100 60" version="1.1" xml:space="preserve" enable-background="new 0 0 100 60"><title>t</title><desc>d</desc><metadata><x xmlns="urn:x"><y/></x></metadata><sodipodi:namedview pagecolor="#ffffff"/><g inkscape:label="Layer 1" inkscape:groupmode="layer" class="layer"><rect class="a" width="100" height="60" fill="#1B2A4A"/><circle cx="50" cy="30" r="20" fill="#3FCF8E"/></g></svg>`],
  ];
}

export function svgGateCases(): SvgCase[] {
  const docs = [...sizing(), ...transforms(), ...gradients(), ...clips(), ...misc()];
  return docs.flatMap(([id, doc, sizes]) => (sizes ?? ANALYTIC_SIZES).map(([w, h]) => ({ id: `gate-${id}-${w}x${h}`, kind: 'svg' as const, svg: doc, width: w, height: h })));
}
