/* The native font registry in main (native engine plan, section 12), behind
   the POWERMOVE_ENGINE_FONTS=1 flag.

   Before Phase 2, main hosts the native registry through the pm-bridge addon
   (engine/crates/pm-bridge, `FontRegistry`) and serves pm-wasm in the
   renderer: batched registry answers for the layout's misses, metrics-and-
   shaping subsets for the faces those answers name, the level 2 fallback
   manifest, and project fonts. Answers, manifests, and face lists cross as
   pm-text's JSON strings and go into pm-wasm untouched.

   With the flag off (the default) nothing loads and no channel is
   registered. With the flag on and no addon, main logs why and carries on
   without it. Development builds find the addon at
   engine/target/pm-bridge/pm_bridge.node (engine/crates/pm-bridge/scripts/
   build.sh); POWERMOVE_PM_BRIDGE overrides the path. */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

export const ENGINE_FONTS_FLAG = 'POWERMOVE_ENGINE_FONTS';
export const ENGINE_BRIDGE_PATH = 'POWERMOVE_PM_BRIDGE';

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
  generation: 'engine-fonts:generation'
} as const;

export const ENGINE_FONTS_LIMITS = {
  queryJsonChars: 4 * 1024 * 1024,
  familyChars: 512,
  descriptorChars: 128,
  /** Large CJK collections run to about 70 MB. */
  fontBytes: 128 * 1024 * 1024,
  faceIndex: 0xffff
} as const;

export interface FontFaceDescriptors {
  weight?: string;
  style?: string;
  stretch?: string;
}

/** pm-bridge's `FontRegistry`. */
export interface NativeFontRegistry {
  readonly generation: number;
  familyNames(): string[];
  familyFaces(family: string): string;
  fallbackManifest(baseFamily: string): string;
  addProjectFont(family: string, data: Uint8Array, descriptors?: FontFaceDescriptors, index?: number): void;
  setFontSource(family: string, data: Uint8Array): void;
  clearProjectFonts(): void;
  answer(queries: string): string;
  subset(blob: string, index: number): Uint8Array;
}

export type EngineFontsResult<T> = { ok: true; value: T } | { ok: false; error: string };

export interface EngineFontsContext {
  isTrustedSenderContents(sender: IpcMainInvokeEvent['sender']): boolean;
}

export function engineFontsEnabled(env: NodeJS.ProcessEnv): boolean {
  return env[ENGINE_FONTS_FLAG] === '1';
}

/** Where the addon is: the override, or the development build next to the app. */
export function bridgeAddonPath(env: NodeJS.ProcessEnv, appPath: string): string {
  return env[ENGINE_BRIDGE_PATH] || path.resolve(appPath, '../../engine/target/pm-bridge/pm_bridge.node');
}

/** Loads the addon and creates the registry (on main's thread, as CoreText matching requires). */
export function loadNativeFontRegistry(file: string, load: (file: string) => unknown = createRequire(__filename)): NativeFontRegistry {
  const addon = load(file) as { FontRegistry?: new () => NativeFontRegistry };
  if (typeof addon?.FontRegistry !== 'function') throw new Error(`${file} does not export FontRegistry`);
  return new addon.FontRegistry();
}

const BLOB_KEY = /^[0-9a-f]{32}$/;

function isText(value: unknown, max: number): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= max;
}

function isIndex(value: unknown): value is number {
  return Number.isInteger(value) && (value as number) >= 0 && (value as number) <= ENGINE_FONTS_LIMITS.faceIndex;
}

function fontBytes(value: unknown): Uint8Array | null {
  const bytes = value instanceof ArrayBuffer ? new Uint8Array(value) : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : null;
  return bytes && bytes.byteLength > 0 && bytes.byteLength <= ENGINE_FONTS_LIMITS.fontBytes ? bytes : null;
}

function descriptors(value: unknown): FontFaceDescriptors | null {
  if (value === undefined || value === null) return {};
  if (typeof value !== 'object' || Array.isArray(value)) return null;
  const out: FontFaceDescriptors = {};
  for (const key of ['weight', 'style', 'stretch'] as const) {
    const v = (value as Record<string, unknown>)[key];
    if (v === undefined) continue;
    if (typeof v !== 'string' || v.length > ENGINE_FONTS_LIMITS.descriptorChars) return null;
    out[key] = v;
  }
  return out;
}

function run<T>(fn: () => T): EngineFontsResult<T> {
  try {
    return { ok: true, value: fn() };
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
}

const invalid = (what: string): EngineFontsResult<never> => ({ ok: false, error: `invalid ${what}` });

/** Registers the registry's channels. Untrusted senders get null. */
export function registerEngineFontsIpc(ipc: Pick<IpcMain, 'handle'>, ctx: EngineFontsContext, registry: NativeFontRegistry): void {
  const handle = (channel: string, fn: (...args: unknown[]) => EngineFontsResult<unknown>) =>
    ipc.handle(channel, (event, ...args: unknown[]) => (ctx.isTrustedSenderContents(event.sender) ? fn(...args) : null));

  handle(ENGINE_FONTS_IPC.answer, queries => (isText(queries, ENGINE_FONTS_LIMITS.queryJsonChars) ? run(() => registry.answer(queries)) : invalid('queries')));
  handle(ENGINE_FONTS_IPC.subset, (blob, index) => (typeof blob === 'string' && BLOB_KEY.test(blob) && isIndex(index) ? run(() => registry.subset(blob, index)) : invalid('subset key')));
  handle(ENGINE_FONTS_IPC.manifest, base => (isText(base, ENGINE_FONTS_LIMITS.familyChars) ? run(() => registry.fallbackManifest(base)) : invalid('family')));
  handle(ENGINE_FONTS_IPC.familyFaces, family => (isText(family, ENGINE_FONTS_LIMITS.familyChars) ? run(() => registry.familyFaces(family)) : invalid('family')));
  handle(ENGINE_FONTS_IPC.addProjectFont, (family, data, desc, index) => {
    const bytes = fontBytes(data);
    const d = descriptors(desc);
    if (!isText(family, ENGINE_FONTS_LIMITS.familyChars) || !bytes || !d || (index !== undefined && !isIndex(index))) return invalid('project font');
    return run(() => {
      registry.addProjectFont(family, bytes, d, index as number | undefined);
      return registry.generation;
    });
  });
  handle(ENGINE_FONTS_IPC.setFontSource, (family, data) => {
    const bytes = fontBytes(data);
    if (!isText(family, ENGINE_FONTS_LIMITS.familyChars) || !bytes) return invalid('font source');
    return run(() => {
      registry.setFontSource(family, bytes);
      return registry.generation;
    });
  });
  handle(ENGINE_FONTS_IPC.clearProjectFonts, () =>
    run(() => {
      registry.clearProjectFonts();
      return registry.generation;
    })
  );
  handle(ENGINE_FONTS_IPC.generation, () => run(() => registry.generation));
}

export interface EngineFontsSetup {
  ipc: Pick<IpcMain, 'handle'>;
  ctx: EngineFontsContext;
  env: NodeJS.ProcessEnv;
  appPath: string;
  log?: (message: string) => void;
  load?: (file: string) => unknown;
}

/** Behind the flag: loads the registry and registers its channels. Returns the registry, or null when off or unavailable. */
export function setupEngineFonts({ ipc, ctx, env, appPath, log = console.warn, load }: EngineFontsSetup): NativeFontRegistry | null {
  if (!engineFontsEnabled(env)) return null;
  const file = bridgeAddonPath(env, appPath);
  if (!existsSync(file)) {
    log(`[engine-fonts] ${ENGINE_FONTS_FLAG}=1 but the pm-bridge addon is missing at ${file} (engine/crates/pm-bridge/scripts/build.sh builds it)`);
    return null;
  }
  try {
    const registry = loadNativeFontRegistry(file, load);
    registerEngineFontsIpc(ipc, ctx, registry);
    return registry;
  } catch (error) {
    log(`[engine-fonts] could not load the font registry from ${file}: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  }
}
