import type { ToolFamily } from './activity-rows';

/* 24-box stroke glyphs for tool rows, one per family. Kept as path data so
   the Svelte row can render them without an icon registry round-trip; the
   `fill` flag marks the sparkle, which is a filled shape. */
export interface ToolGlyph {
  d: string;
  fill?: boolean;
}

const GLYPHS: Record<ToolFamily, ToolGlyph> = {
  think: { d: 'M12 2l2.4 7.2L22 12l-7.6 2.8L12 22l-2.4-7.2L2 12l7.6-2.8z', fill: true },
  run: { d: 'M4 17l6-5-6-5M12 19h8' },
  edit: { d: 'M17 3a2.8 2.8 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z' },
  read: { d: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6' },
  search: { d: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-4.3-4.3' },
  image: { d: 'M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21' },
  computer: { d: 'M2 5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2zM8 21h8M12 17v4' },
  panel: { d: 'M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM9 3v18M3 9h6' },
  tool: { d: 'M14.7 6.3a4 4 0 0 0 5 5l-9.4 9.4a2.1 2.1 0 0 1-3-3l9.4-9.4zM21 3l-3.3 3.3' }
};

export function toolGlyph(family: ToolFamily): ToolGlyph {
  return GLYPHS[family] ?? GLYPHS.tool;
}
