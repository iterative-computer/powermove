/*
 * The font registry's channels between pm-wasm's text layout in the renderer
 * and main (native engine plan, section 12; docs/engine/text.md). With "Use
 * New Engine" on, main relays them to the interactive engine's font service;
 * with it off and POWERMOVE_ENGINE_FONTS=1, main's own registry answers.
 * Without either, nothing answers and pm-wasm keeps the legacy text path.
 */

export const ENGINE_FONTS_IPC = {
  /** `Query[]` JSON → `{ generation, faces, answers }` JSON. */
  answer: 'engine-fonts:answer',
  /** `(blob, index)` → the face's subset bytes. */
  subset: 'engine-fonts:subset',
  /** Base family → `FallbackManifest` JSON (level 2). */
  manifest: 'engine-fonts:manifest',
  /** Family → `FaceSource[]` JSON (level 2). */
  familyFaces: 'engine-fonts:family-faces',
  addProjectFont: 'engine-fonts:add-project-font',
  setFontSource: 'engine-fonts:set-font-source',
  clearProjectFonts: 'engine-fonts:clear-project-fonts',
  generation: 'engine-fonts:generation',
  /** → `{ seeds: FontCacheSeed[] }` JSON: the asking window's renderer font
   *  cache seeds (shared/engine-fonts-seeds.ts). */
  seeds: 'engine-fonts:seeds'
} as const;

/** Main → renderer messages on the font channels. */
export const ENGINE_FONTS_EVENTS = {
  /** The window's font cache seeds changed (`{ seeds }` JSON). */
  seedsChanged: 'engine-fonts:seeds-changed'
} as const;

export type EngineFontsResult<T> = { ok: true; value: T } | { ok: false; error: string };

/** A project font's `FontFace` descriptors, as CSS strings. */
export interface FontFaceDescriptors {
  weight?: string;
  style?: string;
  stretch?: string;
}

/** The renderer's view (`window.powermove.engineFonts`, desktop host). Every
 *  call resolves to a result; a channel nobody registered rejects. */
export interface EngineFontsBridge {
  answer(queries: string): Promise<EngineFontsResult<string>>;
  subset(blob: string, index: number): Promise<EngineFontsResult<Uint8Array>>;
  manifest(base: string): Promise<EngineFontsResult<string>>;
  familyFaces(family: string): Promise<EngineFontsResult<string>>;
  addProjectFont(family: string, data: Uint8Array, descriptors?: FontFaceDescriptors, index?: number): Promise<EngineFontsResult<number>>;
  setFontSource(family: string, data: Uint8Array): Promise<EngineFontsResult<number>>;
  clearProjectFonts(): Promise<EngineFontsResult<number>>;
  generation(): Promise<EngineFontsResult<number>>;
  /** This window's font cache seeds (`{ seeds }` JSON). */
  seeds?(): Promise<EngineFontsResult<string>>;
  /** Calls `listener` with the window's seeds whenever they change. */
  onSeedsChanged?(listener: (json: string) => void): () => void;
}
