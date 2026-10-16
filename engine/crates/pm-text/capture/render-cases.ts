/* pm-text rendering corpus: text rasters and caption rasters drawn by
   today's Canvas2D code (render-page.ts), compared with the engine's glyph
   rendering (`pm-render` tests/text_raster.rs).

   Usage: bun engine/crates/pm-text/capture/render-cases.ts <out.json>

   Only fonts that ship with macOS are named; the corpus file is safe to
   keep in the repository. Captured pixels are not (they are renders of
   Apple's fonts) and stay in the private corpus directory. */

import { PROJECT_FONTS, VARIABLE_SOURCES, type TextContent } from './corpus';

export interface TextCase {
  id: string;
  group: string;
  kind: 'text';
  /** Resolved text content, including `color`. */
  d: TextContent & { color: string };
  /** `continuousRasterScale` of the layer: the raster density request. */
  scale: number;
}

export interface CaptionCase {
  id: string;
  group: string;
  kind: 'caption';
  content: { cues: any[]; style: any };
  /** Layer-local time. */
  local: number;
  compositionWidth: number;
  compositionHeight: number;
  scale: number;
}

export type RenderCase = TextCase | CaptionCase;

const PANGRAM = 'The quick brown fox jumps over the lazy dog 0123';
const KERNING = 'AVATAR Tower WAVE fi fl ffi';
const PARAGRAPH = 'Motion design brings graphics to life. Powermove composes titles, captions, and shapes on a timeline.';

const SCRIPTS: Record<string, string> = {
  emoji: 'Hi 👋🏽 team 👨‍👩‍👧‍👦 🇰🇷 ❤️ 1️⃣',
  emojiOnly: '🎬🎥🍿😀',
  cjkSc: '视频编辑软件让创作更简单。',
  cjkJa: '動画編集はとても楽しい カタカナ',
  korean: '한국어 텍스트 레이아웃',
  arabic: 'مرحبا بالعالم 123 في Powermove',
  hebrew: 'שלום עולם 2026',
  devanagari: 'नमस्ते दुनिया क्षत्रिय',
  thai: 'สวัสดีชาวโลก ภาษาไทย',
  mixed: 'Edit 编辑 편집 تحرير ✂️ done',
};

function text(partial: Partial<TextContent> & { text: string; size: number; color?: string }): TextCase['d'] {
  const boxWidth = Math.max(0, partial.boxWidth ?? 0);
  return {
    font: 'SF Pro Display', weight: 600, tracking: 0, leading: 1.15, align: 'left', italic: false,
    boxHeight: 0, color: '#FFFFFF', ...partial, boxWidth, paragraph: boxWidth > 0,
  } as TextCase['d'];
}

const BASE_STYLE = {
  preset: 'classic', font: 'SF Pro Display', weight: 600, italic: false, size: 54, tracking: 0, leading: 1.2,
  fill: '#FFFFFF', stroke: 3, strokeColor: '#000000', box: false, boxColor: '#000000', boxOpacity: 72,
  boxPadding: 14, boxRadius: 8, placement: 'bottom', align: 'center', offsetX: 0, offsetY: 0, safeMargin: 8,
  maxWidth: 80, maxLines: 2, textCase: 'original', highlight: false, highlightColor: '#FFD43B',
  highlightScale: 100, wordAnimation: 'none',
};

// The six presets of captions/model.ts, at 1080p.
const PRESETS: Record<string, any> = {
  classic: {},
  boxed: { weight: 500, size: 48, stroke: 0, box: true, boxColor: '#000000', boxOpacity: 72, boxPadding: 14, boxRadius: 10 },
  whisper: { weight: 400, size: 38, tracking: 0.5, leading: 1.3, stroke: 2, strokeColor: '#00000099', maxWidth: 70, maxLines: 1, safeMargin: 7 },
  spotlight: { weight: 800, size: 64, leading: 1.1, stroke: 4, placement: 'middle', offsetY: 220, maxWidth: 70, highlight: true, highlightColor: '#24D5FF', highlightScale: 108 },
  pop: { weight: 900, size: 82, leading: 1.05, stroke: 7, placement: 'middle', offsetY: 160, maxWidth: 76, maxLines: 2, textCase: 'upper', highlight: true, highlightColor: '#FFD43B', highlightScale: 116, wordAnimation: 'pop' },
  paper: { weight: 500, size: 46, leading: 1.25, fill: '#151515', stroke: 0, box: true, boxColor: '#FFFFFF', boxOpacity: 94, boxPadding: 16, boxRadius: 6 },
};

function cue(textValue: string, start = 0, end = 3) {
  const words = textValue.split(/\s+/).filter(Boolean);
  const step = (end - start) / words.length;
  return {
    id: 'c1', text: textValue, start, end,
    words: words.map((w, i) => ({ text: w, start: start + i * step, end: start + (i + 1) * step })),
  };
}

export function buildRenderCases(): RenderCase[] {
  const cases: RenderCase[] = [];
  const addText = (group: string, d: TextCase['d'], scale = 1) =>
    cases.push({ id: `${group}-${String(cases.length).padStart(4, '0')}`, group, kind: 'text', d, scale });
  const addCaption = (group: string, style: any, textValue: string, local: number, scale: number, w = 1920, h = 1080) =>
    cases.push({
      id: `${group}-${String(cases.length).padStart(4, '0')}`, group, kind: 'caption',
      content: { cues: [cue(textValue)], style: { ...BASE_STYLE, ...style } }, local,
      compositionWidth: w, compositionHeight: h, scale,
    });

  // Sizes 8 to 200 px in the default face.
  for (const size of [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 28, 32, 40, 48, 64, 80, 96, 128, 160, 200]) {
    addText('sizes', text({ text: PANGRAM, size }));
    addText('sizes-regular', text({ text: PANGRAM, size, weight: 400 }));
  }
  // Families at small, text, and display sizes.
  const families = ['Helvetica Neue', 'Avenir Next', 'Georgia', 'Times New Roman', 'Menlo', 'New York', 'Arial',
    'Futura', 'Gill Sans', 'Baskerville', 'Didot', 'Courier New', 'Impact', 'Noteworthy', 'Marker Felt', 'System Font', 'SF Pro Text'];
  for (const font of families) {
    for (const size of [11, 14, 18, 24, 48, 96]) addText('families', text({ text: PANGRAM, font, size, weight: 400 }));
  }
  // Weights.
  for (const font of ['SF Pro Display', 'Helvetica Neue', 'Avenir Next']) {
    for (const weight of [100, 200, 300, 400, 500, 600, 700, 800, 900]) {
      for (const size of [12, 16, 32, 72]) addText('weights', text({ text: KERNING, font, size, weight }));
    }
  }
  // Italics, real and synthetic.
  for (const font of ['SF Pro Display', 'Georgia', 'New York', 'Helvetica Neue']) {
    for (const size of [13, 18, 40, 96]) addText('italic', text({ text: PANGRAM, font, size, weight: 400, italic: true }));
  }
  // Synthetic bold and italic: a project family with only a regular face.
  for (const size of [14, 36, 80]) {
    addText('synthetic', text({ text: PANGRAM, font: 'Chalkduster', size, weight: 700 }));
    addText('synthetic', text({ text: PANGRAM, font: 'Chalkduster', size, weight: 400, italic: true }));
  }
  // Variable fonts through the FontFace variation path.
  const variable: Array<[string, Record<string, number>]> = [
    ['System Font', { wght: 100 }], ['System Font', { wght: 400 }], ['System Font', { wght: 1000 }],
    ['System Font', { wght: 700, wdth: 30 }], ['System Font', { wdth: 150 }],
    ['Skia', { wght: 0.48 }], ['Skia', { wght: 3.2 }], ['Skia', { wght: 1, wdth: 0.62 }],
    ['STIX Two Text', { wght: 700 }],
  ];
  for (const [font, axes] of variable) {
    for (const size of [14, 48, 120]) {
      const d: any = text({ text: KERNING, font, size, weight: 400 });
      for (const [tag, value] of Object.entries(axes)) d[`fontAxis.${tag}`] = value;
      addText('variable', d);
    }
  }
  // Scripts, emoji, RTL.
  for (const [key, value] of Object.entries(SCRIPTS)) {
    for (const size of key.startsWith('emoji') ? [12, 16, 20, 24, 32, 40, 48, 64, 96, 160] : [14, 24, 48, 96]) {
      addText(key.startsWith('emoji') ? 'emoji' : 'scripts', text({ text: value, size, weight: 500 }));
    }
  }
  for (const font of ['Geeza Pro', 'Arial Hebrew', 'PingFang SC', 'Hiragino Sans', 'Apple SD Gothic Neo']) {
    const value = font === 'Geeza Pro' ? SCRIPTS.arabic! : font === 'Arial Hebrew' ? SCRIPTS.hebrew! : font === 'Apple SD Gothic Neo' ? SCRIPTS.korean! : SCRIPTS.cjkJa!;
    for (const size of [14, 40]) addText('scripts', text({ text: value, font, size, weight: 400 }));
  }
  // Tracking, leading, alignment, paragraphs.
  for (const tracking of [-2, 0.5, 3, 12]) {
    for (const size of [16, 48]) addText('tracking', text({ text: KERNING, size, tracking }));
  }
  for (const align of ['left', 'center', 'right'] as const) {
    addText('paragraph', text({ text: PARAGRAPH, size: 24, weight: 500, align, boxWidth: 420, leading: 1.3 }));
    addText('paragraph', text({ text: 'Line one\nLine two is longer\n\nLine four', size: 36, align, leading: 1 }));
  }
  // Colors: Skia's glyph masks depend on the paint's luminance.
  for (const color of ['#000000', '#FF3B30', '#808080', '#24D5FF', '#151515', '#FFD43B', '#FFFFFF80']) {
    for (const size of [12, 16, 48]) addText('colors', text({ text: PANGRAM, size, color }));
  }
  // Raster densities (continuousRasterScale quarter steps).
  for (const scale of [0.25, 0.5, 0.75, 1.25, 1.5, 2, 2.75, 4]) {
    for (const size of [12, 24, 48]) addText('density', text({ text: KERNING, size }), scale);
  }
  // Glyphs above Skia's 256 px atlas limit draw as paths.
  addText('large', text({ text: 'Big Ag', size: 200 }), 1.5);
  addText('large', text({ text: 'Mega', size: 300, weight: 800 }));

  // Captions: every preset, word states at several times, two scales.
  const line = 'Every caption style renders its words';
  for (const [name, style] of Object.entries(PRESETS)) {
    for (const scale of [0.5, 1]) {
      for (const local of [0.2, 1.3, 2.9]) addCaption(`caption-${name}`, style, line, local, scale);
    }
  }
  // Stroked captions across sizes and stroke widths (amendment 10).
  for (const size of [20, 28, 38, 54, 72, 96]) {
    for (const stroke of [1, 2, 4, 7]) addCaption('caption-stroke', { size, stroke }, 'Stroked caption text', 1, size <= 28 ? 1 : 0.5);
  }
  addCaption('caption-stroke', { size: 54, stroke: 3, strokeColor: '#00000099' }, 'Translucent stroke', 1, 1);
  addCaption('caption-stroke', { size: 54, stroke: 5, strokeColor: '#FF3B30', fill: '#000000' }, 'Colored stroke', 1, 1);
  // Caption fades and scripts.
  addCaption('caption-fade', { wordAnimation: 'fade' }, line, 0.9, 1);
  addCaption('caption-scripts', { stroke: 2 }, 'Hello 👋 世界 مرحبا', 1, 1);
  addCaption('caption-scripts', { textCase: 'upper', font: 'Avenir Next', weight: 700 }, 'avenir uppercase', 1, 0.75);
  return cases;
}

/** The fitting set (`--set fit`): the profiles are fitted on it and
    validated on the main set, which it does not share cases with. */
export function buildFitCases(): RenderCase[] {
  const cases: RenderCase[] = [];
  const add = (group: string, d: TextCase['d'], scale = 1) =>
    cases.push({ id: `${group}-${String(cases.length).padStart(4, '0')}`, group, kind: 'text', d, scale });
  const families = ['SF Pro Display', 'Helvetica Neue', 'Avenir Next', 'Georgia', 'Times New Roman', 'Menlo',
    'Arial', 'Futura', 'New York', 'Gill Sans'];
  const sizes = [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 28, 32, 40, 48, 64, 96, 128, 160, 200, 240];
  for (const font of families) {
    for (const size of sizes) {
      for (const weight of [400, 700]) add('fit', text({ text: 'Hamburgefonstiv 0123', font, size, weight }));
    }
  }
  // Paint luminance (Skia's glyph masks depend on it).
  for (const color of ['#000000', '#404040', '#808080', '#C0C0C0', '#FFFFFF', '#FF0000', '#00FF00', '#0000FF', '#FFFF00', '#00FFFF']) {
    for (const size of [10, 12, 14, 18, 24, 48]) add('fit-color', text({ text: 'Hamburgefonstiv 0123', size, color }));
  }
  // Emoji placement and scale by size.
  for (const value of ['😀', '🇰🇷']) {
    for (let size = 6; size <= 64; size++) add('fit-emoji', text({ text: value, size, weight: 500 }));
  }
  // Caption strokes.
  const caps: RenderCase[] = [];
  for (const size of [16, 20, 24, 32, 40, 54, 72]) {
    for (const stroke of [1, 2, 3, 4, 6, 8]) {
      caps.push({
        id: `fit-stroke-${String(cases.length + caps.length).padStart(4, '0')}`, group: 'fit-stroke', kind: 'caption',
        content: { cues: [cue('Stroked words')], style: { ...BASE_STYLE, size, stroke } }, local: 1,
        compositionWidth: 1920, compositionHeight: 1080, scale: 1,
      });
    }
  }
  return [...cases, ...caps];
}

/** Edge probes (`--set probe`): Menlo's full block (a rectangle) at many
    sizes, so edges cross every subpixel phase; the plain coverage of a
    rectangle is known exactly, which separates the smoothing's dilation
    from its coverage curve. */
export function buildProbeCases(): RenderCase[] {
  const cases: RenderCase[] = [];
  const add = (group: string, d: TextCase['d'], scale = 1) =>
    cases.push({ id: `${group}-${String(cases.length).padStart(4, '0')}`, group, kind: 'text', d, scale });
  for (const color of ['#FFFFFF', '#000000']) {
    for (let size = 8; size <= 40; size += color === '#FFFFFF' ? 0.25 : 1) {
      add(color === '#FFFFFF' ? 'probe-white' : 'probe-black', text({ text: '\u2588 \u2588 \u2588 \u2588', font: 'Menlo', size, weight: 400, color }));
    }
  }
  // Where Skia stops drawing glyph masks: a short glyph and a tall one.
  for (let size = 150; size <= 300; size += 10) {
    add('probe-large-h', text({ text: 'H', font: 'Menlo', size, weight: 400 }));
    add('probe-large-block', text({ text: '\u2588', font: 'Menlo', size, weight: 400 }));
  }
  return cases;
}

if (import.meta.main) {
  const out = process.argv[2];
  if (!out) throw new Error('usage: bun render-cases.ts <out.json> [fit]');
  const set = process.argv[3];
  const cases = set === 'fit' ? buildFitCases() : set === 'probe' ? buildProbeCases() : buildRenderCases();
  await Bun.write(out, JSON.stringify({ variableSources: VARIABLE_SOURCES, projectFonts: PROJECT_FONTS, cases }));
  const groups = new Map<string, number>();
  for (const item of cases) groups.set(item.group, (groups.get(item.group) ?? 0) + 1);
  console.log(`${cases.length} cases`, Object.fromEntries(groups));
}
