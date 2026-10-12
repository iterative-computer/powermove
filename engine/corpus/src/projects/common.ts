import type { Builder, Doc, Layer } from '../doc';
import { ref } from '../media';

export interface CorpusProject {
  /** Stable slug, also the capture directory name: `<group>/<name>`. */
  id: string;
  /** Appendix B heading the project belongs to. */
  group: string;
  title: string;
  /** Appendix B features the project exercises. */
  covers: string[];
  doc: Doc;
  /** Composition times (seconds) to capture. */
  samples: number[];
  /** Also capture transparent renders (float RGBA and straight-alpha PNG). */
  alpha?: boolean;
  /** Capture size when it should differ from the composition size. */
  size?: [number, number];
  notes?: string;
  /** e2e fixture extensions (apps/desktop/e2e/fixtures/<name>) installed in the capture profile. */
  extensions?: string[];
  /**
   * `render3d` settings for the Blender snapshot pass (`enabled` is forced on there).
   * Documents keep Blender output off so the captured frames are the native renderer's.
   */
  blender?: Record<string, unknown>;
}

export function project(b: Builder, fields: Omit<CorpusProject, 'id' | 'doc'>): CorpusProject {
  return { id: b.slug, doc: b.root, ...fields };
}

export const GRADIENT = {
  type: 'linear', angle: 35,
  stops: [
    { id: 'stop-1', color: '#0C0D12', position: 0 },
    { id: 'stop-2', color: '#1B2A4A', position: 55 },
    { id: 'stop-3', color: '#3A2A5A', position: 100 },
  ],
};

/**
 * A shared test card: the generated SVG artwork, a hard-edged shape row, and
 * a text line, so effects and compositing operate on edges, gradients, text,
 * and flat color at once. Returns the layers bottom-first as added.
 */
export function testCard(b: Builder, options: { prefix?: string } = {}): Layer[] {
  const prefix = options.prefix ?? '';
  const comp = b.comp;
  const art = b.add(b.layer('image', {
    name: `${prefix}Artwork`, d: { asset: ref('corpus-art.svg'), fit: 'cover', w: comp.w * .7, h: comp.h * .7 },
    p: { 'position.x': comp.w * .42, 'position.y': comp.h * .45 },
  }));
  const colors = ['#FF6B1A', '#4C8DFF', '#3FCF8E', '#F2C14E', '#E8E2CF'];
  const chips = colors.map((color, index) => b.add(b.layer('shape', {
    name: `${prefix}Chip ${index + 1}`,
    d: { shape: index % 2 ? 'ellipse' : 'rect', color, w: comp.w * .08, h: comp.w * .08, radius: 6 },
    p: { 'position.x': comp.w * (.15 + index * .1), 'position.y': comp.h * .84, rotation: index * 9 },
  })));
  const text = b.add(b.layer('text', {
    name: `${prefix}Title`,
    d: { text: 'Powermove 1.0', size: Math.round(comp.h * .11), weight: 700, color: '#FFFFFF', align: 'left', tracking: 0 },
    p: { 'position.x': comp.w * .66, 'position.y': comp.h * .22 },
  }));
  return [art, ...chips, text];
}
