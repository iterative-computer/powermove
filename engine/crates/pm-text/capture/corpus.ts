/* pm-text level 1 layout corpus (the S4 corpus plus project fonts).
   Deterministic list of text layer contents (`layer.d` after
   `resolvedTextContent`) covering families, weights, sizes 9 to 400, tracking,
   leading, box widths, scripts, emoji, variable axes, UTF-16 splits, and
   project fonts (`FontFace`s a document registers).

   Usage: bun engine/crates/pm-text/capture/corpus.ts <out.json>

   Only fonts that ship with macOS are named here; nothing from a user's font
   folder is referenced, so the corpus is safe to keep in a public repository. */

export interface TextContent {
  text: string;
  font?: string;
  weight?: number;
  size: number;
  tracking?: number;
  leading?: number;
  align?: 'left' | 'center' | 'right';
  italic?: boolean;
  boxWidth: number;
  boxHeight: number;
  paragraph: boolean;
  [axis: `fontAxis.${string}`]: number;
}

export interface CorpusCase { id: string; group: string; d: TextContent }

/** A project font: a `FontFace(family, bytes, {weight, style})` the document adds. */
export interface ProjectFont { family: string; path: string; weight: string; style: 'normal' | 'italic' }

/** Variable-font sources (`PM.fontSource(family)`, the `packagedSource` hook). */
export const VARIABLE_SOURCES: Record<string, string> = {
  'Skia': '/System/Library/Fonts/Supplemental/Skia.ttf',
  'STIX Two Text': '/System/Library/Fonts/Supplemental/STIXTwoText.ttf',
  'Kefa III': '/System/Library/Fonts/Supplemental/KefaIII.ttf',
  'Noto Sans Syriac': '/System/Library/Fonts/Supplemental/NotoSansSyriac-Regular.ttf',
  'System Font': '/System/Library/Fonts/SFNS.ttf',
};

export const PROJECT_FONTS: ProjectFont[] = [
  { family: 'PM Project Serif', path: '/System/Library/Fonts/Supplemental/Georgia.ttf', weight: '400', style: 'normal' },
  { family: 'PM Project Serif', path: '/System/Library/Fonts/Supplemental/Georgia Bold.ttf', weight: '700', style: 'normal' },
  { family: 'PM Project Serif', path: '/System/Library/Fonts/Supplemental/Georgia Italic.ttf', weight: '400', style: 'italic' },
  { family: 'PM Project Variable', path: '/System/Library/Fonts/Supplemental/STIXTwoText.ttf', weight: '400 700', style: 'normal' },
  { family: 'PM Project Variable', path: '/System/Library/Fonts/Supplemental/STIXTwoText-Italic.ttf', weight: '400 700', style: 'italic' },
  // Shadows an installed family: project fonts win over installed ones.
  { family: 'Chalkduster', path: '/System/Library/Fonts/Supplemental/Georgia.ttf', weight: '400', style: 'normal' },
];

export interface Corpus { variableSources: Record<string, string>; projectFonts: ProjectFont[]; cases: CorpusCase[] }

const LATIN = {
  word: 'Powermove',
  pangram: 'The quick brown fox jumps over the lazy dog.',
  kerning: 'AVATAR Tower WAVE Yo To Ta LT AY fi fl ffi ff',
  digits: '0123456789 $1,234.56 -42% 3/4',
  punct: '“Quotes” — dashes – and ‘apostrophes’… (parens) [brackets] {braces} ¿¡',
  caps: 'POWERMOVE MOTION DESIGN',
  mixedCase: 'iPhone macOS WWDC Ångström façade naïve',
  accents: 'Ŝ̂ ẞ ǅ ǈ é ǟ Z̵͔',
};

const SCRIPTS = {
  emoji: 'Hi 👋🏽 team 👨‍👩‍👧‍👦 🇰🇷 🏳️‍🌈 ❤️ 1️⃣',
  emojiOnly: '🎬🎥📽️🎞️🍿',
  cjkSc: '视频编辑软件让创作更简单。',
  cjkJa: '動画編集はとても楽しいです。カタカナとひらがな',
  korean: '한국어 텍스트 레이아웃 테스트입니다',
  arabic: 'مرحبا بالعالم 123 في Powermove',
  hebrew: 'שלום עולם 2026',
  devanagari: 'नमस्ते दुनिया क्षत्रिय',
  thai: 'สวัสดีชาวโลก ภาษาไทย',
  mixed: 'Edit 编辑 編集 편집 تحرير ✂️ done',
  symbols: '→ ← ↑ ↓ ⌘ ⌥ ⇧ ✓ ★ ♫ ∑ ∞ ≠ ≤',
};

const PARAGRAPH = 'Motion design is the art of bringing graphics to life. Powermove lets editors compose titles, captions, and shapes on a timeline with precise control over every frame.';
const LONG_WORD = 'Supercalifragilisticexpialidocious-antidisestablishmentarianism';

const FAMILIES = [
  'SF Pro Display', 'SF Pro Text', 'SF Mono', 'Helvetica Neue', 'Helvetica', 'Arial', 'Arial Black',
  'Arial Narrow', 'Avenir', 'Avenir Next', 'Futura', 'Gill Sans', 'Optima', 'Georgia', 'Times New Roman',
  'Baskerville', 'Didot', 'American Typewriter', 'Courier New', 'Menlo', 'Monaco', 'Andale Mono', 'Impact',
  'New York', 'Hoefler Text', 'Palatino', 'Charter', 'Rockwell', 'Copperplate', 'Marker Felt',
  'Noteworthy', 'Chalkboard SE', 'Big Caslon', 'Iowan Old Style', 'Trebuchet MS', 'Verdana', 'Tahoma',
  'Lucida Grande', 'Geneva', 'Comic Sans MS', 'Not An Installed Family',
  // The hidden system font answers to its name-table family "System Font" in
  // Chromium (SF Pro Display, SF Pro Text, and -apple-system do not).
  'System Font', 'system-ui',
];

const CJK_FAMILIES = ['PingFang SC', 'Hiragino Sans', 'Hiragino Mincho ProN', 'Apple SD Gothic Neo', 'Songti SC'];
const RTL_FAMILIES = ['Geeza Pro', 'Damascus', 'Arial Hebrew', 'Noto Nastaliq Urdu'];

const SIZES = [9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 96, 112, 128, 144, 160, 200, 240, 288, 320, 400];
const TRACKING = [-10, -2, -0.5, 0, 0.25, 1, 3, 12, 40];
const LEADING = [0.7, 1, 1.1, 1.15, 1.5, 2.25];

function content(partial: Partial<TextContent> & { text: string; size: number }): TextContent {
  const boxWidth = Math.max(0, partial.boxWidth ?? 0);
  return {
    font: 'SF Pro Display', weight: 600, tracking: 0, leading: 1.15, align: 'left', italic: false,
    boxHeight: 0, ...partial, boxWidth, paragraph: boxWidth > 0,
  } as TextContent;
}

export function buildCorpus(): CorpusCase[] {
  const cases: CorpusCase[] = [];
  const add = (group: string, d: TextContent) => cases.push({ id: `${group}-${String(cases.length).padStart(5, '0')}`, group, d });

  // Today's default text layer, every size.
  for (const size of SIZES) {
    add('default', content({ text: 'Powermove', size, weight: 600, tracking: -2, leading: 1.1, align: 'center' }));
    add('default', content({ text: LATIN.pangram, size, weight: 600, tracking: -2, leading: 1.1 }));
  }

  // Families: a few sizes, several Latin samples, regular and bold.
  for (const font of FAMILIES) {
    for (const size of [11, 17, 32, 96, 240]) {
      for (const key of ['pangram', 'kerning', 'digits', 'punct', 'mixedCase'] as const) {
        add('families', content({ text: LATIN[key], font, size, weight: 400 }));
      }
      add('families', content({ text: LATIN.caps, font, size, weight: 700 }));
    }
  }

  // System font weights, italics, and optical sizes.
  for (const font of ['SF Pro Display', 'SF Pro Text', 'Helvetica Neue', 'Avenir Next', 'New York', 'SF Mono', 'Georgia', 'System Font']) {
    for (const weight of [100, 200, 300, 400, 500, 600, 700, 800, 900]) {
      for (const italic of [false, true]) {
        for (const size of [9, 13, 17, 20, 28, 44, 64, 96, 144, 400]) {
          add('weights', content({ text: LATIN.kerning, font, size, weight, italic }));
        }
      }
    }
  }

  // Tracking at many sizes (CSS letter-spacing in px, independent of size).
  for (const font of ['SF Pro Display', 'Helvetica Neue', 'Georgia', 'Avenir Next', 'Menlo', 'System Font']) {
    for (const tracking of TRACKING) {
      for (const size of [10, 16, 24, 48, 128, 320]) {
        add('tracking', content({ text: LATIN.kerning, font, size, tracking }));
        add('tracking', content({ text: 'office affinity flow', font, size, tracking }));
      }
    }
  }

  // Leading, alignment, paragraph boxes, box height clipping.
  for (const leading of LEADING) {
    for (const align of ['left', 'center', 'right'] as const) {
      add('leading', content({ text: 'Line one\nLine two is longer\n\nLine four', size: 48, leading, align }));
    }
  }
  const boxes = [1, 24, 60, 100, 150, 220, 320, 480, 800, 1200];
  for (const font of ['SF Pro Display', 'Helvetica Neue', 'Georgia', 'Avenir Next', 'Times New Roman', 'Menlo']) {
    for (const size of [12, 18, 32, 64, 128]) {
      for (const box of boxes) {
        for (const align of ['left', 'center', 'right'] as const) {
          if (align !== 'left' && box % 3 !== 0) continue;
          add('wrap', content({ text: PARAGRAPH, font, size, weight: 500, boxWidth: box * (size / 32), align }));
        }
        add('wrap', content({ text: `${LONG_WORD} ${LATIN.pangram}`, font, size, boxWidth: box * (size / 32) }));
        add('wrap', content({ text: `${PARAGRAPH}\n\n${LATIN.kerning}`, font, size, tracking: 2, boxWidth: box * (size / 24), boxHeight: size * 4.2 }));
      }
    }
  }
  for (const tracking of [-4, 0, 6, 25]) {
    for (const box of [40, 90, 180, 360]) {
      add('wrap', content({ text: PARAGRAPH, size: 36, tracking, boxWidth: box * 1.7, align: 'center' }));
    }
  }

  // Whitespace handling: JS \s set, NBSP, tabs, ideographic space, leading/trailing runs.
  const whitespace = [
    '  leading spaces', 'trailing spaces   ', 'multiple    inner     spaces', 'tab\tseparated\tvalues',
    'no break space here', 'ideographic　space', 'thin and hair', 'line separator',
    'zero​width​space', 'BOM﻿inside', '\n\nleading newlines', 'trailing newline\n', '  \n  \n  ',
  ];
  for (const text of whitespace) {
    for (const box of [0, 70, 160]) add('whitespace', content({ text, size: 28, weight: 400, boxWidth: box }));
  }

  // Scripts with the default font stack and with script-specific families.
  for (const [key, text] of Object.entries(SCRIPTS)) {
    for (const size of [12, 24, 48, 120, 300]) {
      add('scripts', content({ text, size, weight: 500 }));
      add('scripts', content({ text, size, weight: 500, tracking: 3 }));
    }
    for (const box of [60, 140, 300]) add('scripts', content({ text: `${text} ${text}`, size: 32, boxWidth: box }));
    if (key.startsWith('cjk') || key === 'korean') {
      for (const font of CJK_FAMILIES) for (const size of [14, 40, 160]) add('scripts', content({ text, font, size, weight: 400 }));
    }
    if (key === 'arabic' || key === 'hebrew') {
      for (const font of RTL_FAMILIES) for (const size of [14, 40, 160]) add('scripts', content({ text, font, size, weight: 400 }));
    }
  }

  // UTF-16 splits: narrow boxes force the per-code-unit cut loop through surrogate pairs.
  for (const text of [SCRIPTS.emoji, SCRIPTS.emojiOnly, '𝐁𝐨𝐥𝐝 𝕸𝖆𝖙𝖍 𝓢𝓬𝓻𝓲𝓹𝓽', '𠀀𠀁𠀂 rare CJK 𪚥', 'a👍b👍🏿c🇯🇵d']) {
    for (const box of [1, 8, 20, 36, 55, 90]) {
      for (const size of [16, 40]) add('utf16', content({ text, size, weight: 400, boxWidth: box }));
    }
  }

  // Variable fonts with axes (fontAxis.* routes through the FontFace variation path).
  const variable: Array<{ font: string; axes: Record<string, number[]> }> = [
    { font: 'Skia', axes: { wght: [0.48, 1, 1.9, 3.2], wdth: [0.62, 1, 1.3] } },
    { font: 'STIX Two Text', axes: { wght: [400, 550, 700] } },
    { font: 'Kefa III', axes: { wght: [300, 400, 800] } },
    { font: 'Noto Sans Syriac', axes: { wght: [100, 400, 900] } },
    { font: 'System Font', axes: { wght: [100, 400, 700, 1000], wdth: [30, 100, 150] } },
    { font: 'System Font', axes: { opsz: [17, 28, 60, 96] } },
  ];
  for (const { font, axes } of variable) {
    const tags = Object.keys(axes);
    const combos: Array<Record<string, number>> = [{}];
    for (const tag of tags) {
      const next: Array<Record<string, number>> = [];
      for (const combo of combos) for (const value of axes[tag]!) next.push({ ...combo, [tag]: value });
      combos.splice(0, combos.length, ...next);
    }
    for (const combo of combos) {
      for (const size of [14, 48, 160]) {
        const d = content({ text: font === 'Noto Sans Syriac' ? 'ܫܠܡܐ Syriac text' : LATIN.kerning, font, size, weight: 400 });
        for (const [tag, value] of Object.entries(combo)) (d as any)[`fontAxis.${tag}`] = value;
        add('variable', d);
      }
    }
    add('variable', content({ text: LATIN.kerning, font, size: 48, weight: 400 }));
  }

  // Project fonts: CSS font matching across a family's faces, synthetic
  // bold and italic, variable faces with weight ranges, fallback from a web
  // font primary, and a project family shadowing an installed one.
  for (const font of ['PM Project Serif', 'PM Project Variable']) {
    for (const weight of [100, 300, 400, 500, 600, 700, 900]) {
      for (const italic of [false, true]) {
        for (const size of [14, 48, 160]) {
          add('project', content({ text: LATIN.kerning, font, size, weight, italic }));
        }
      }
    }
    add('project', content({ text: `${SCRIPTS.mixed} ${SCRIPTS.emoji}`, font, size: 40, weight: 400 }));
    add('project', content({ text: PARAGRAPH, font, size: 30, weight: 600, tracking: 1.5, boxWidth: 420 }));
  }
  for (const weight of [400, 700]) add('project', content({ text: LATIN.pangram, font: 'Chalkduster', size: 36, weight }));

  // Accents, combining marks, and grapheme clusters for per-character positions.
  for (const size of [20, 64]) {
    add('graphemes', content({ text: LATIN.accents, size, weight: 400 }));
    add('graphemes', content({ text: 'ééé ñ ö̲', size, font: 'Times New Roman', weight: 400 }));
    add('graphemes', content({ text: 'क्षि ष्ट्र', size, weight: 400 }));
  }

  return cases;
}

if (import.meta.main) {
  const out = process.argv[2];
  if (!out) throw new Error('usage: bun corpus.ts <out.json>');
  const cases = buildCorpus();
  const corpus: Corpus = { variableSources: VARIABLE_SOURCES, projectFonts: PROJECT_FONTS, cases };
  await Bun.write(out, JSON.stringify(corpus));
  const groups = new Map<string, number>();
  for (const item of cases) groups.set(item.group, (groups.get(item.group) ?? 0) + 1);
  console.log(`${cases.length} cases`, Object.fromEntries(groups));
}
