import { validateScene, type WebScene } from './scene';
import { zipFiles } from './archive';
import { convertStillImage, readProxyFile } from '../legacy/core/media-conversion';
import { mediaExtension } from '../../../shared/media-formats';
import { agentHandoff } from './agent-handoff';
import { layer3DAssetIds } from '../core/scene3d/layers';
import { sceneAssetIds, parseScene } from '../core/scene3d/schema';

const utf8 = (text: string) => new TextEncoder().encode(text);
const literal = (value: unknown) => JSON.stringify(value).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');

/** Only render source crosses the boundary; workspace, notes and edit history stay in the app. */
function cleanProject(comp: any, cloneFields = true): any {
  const fields = ['id', 'name', 'w', 'h', 'fps', 'dur', 'bg', 'backgroundFill', 'layers', 'assets', 'params', 'shutter', 'colorDepth'];
  const copy = Object.fromEntries(fields.filter(key => comp[key] !== undefined).map(key => [key, cloneFields ? structuredClone(comp[key]) : comp[key]]));
  copy.comps = Object.fromEntries(Object.entries(comp.comps || {}).map(([id, sub]) => [id, cleanProject(sub, cloneFields)]));
  for (const layer of copy.layers) delete layer.locked_intent;
  return copy;
}

export function inspectWebExport(PM: any) {
  const errors = new Set<string>(), warnings = new Set<string>();
  const definitions = { effects: new Map<string, any>(), transitions: new Map<string, any>(), layerTypes: new Map<string, any>() };
  const fonts = new Set<string>(), assets = new Set<string>();
  const inspectChannels = (value: any, name: string) => {
    if (!value || typeof value !== 'object') return;
    if (typeof value.expr === 'string' && value.expr.trim() && !PM.compileExpr(value.expr)) errors.add(`${name}: unsupported expression`);
    for (const child of Object.values(value)) inspectChannels(child, name);
  };
  const getDef = (kind: keyof typeof definitions, id: string, label: string) => {
    const definition = PM.Kernel?.[kind]?.get(id);
    if (!definition) errors.add(`${label}: missing ${kind === 'layerTypes' ? 'layer' : kind} definition “${id}”`);
    else definitions[kind].set(id, definition);
    return definition;
  };
  const visited = new Set<any>(), active = new Set<any>();
  const visit = (comp: any, depth: number) => {
    if (depth > 8) { errors.add('Composition nesting exceeds eight levels'); return; }
    if (active.has(comp)) { errors.add('Nested compositions contain a cycle'); return; }
    if (visited.has(comp)) return;
    visited.add(comp); active.add(comp);
    for (const layer of comp.layers) {
      inspectChannels(layer, layer.name);
      if (layer.d?.asset) assets.add(layer.d.asset);
      if (layer.type === 'text') {
        const addFonts = (value: any) => {
          if (!value || typeof value !== 'object') return;
          for (const [key, child] of Object.entries<any>(value)) {
            if (key === 'font') {
              if (typeof child === 'string') fonts.add(child);
              else { if (typeof child?.v === 'string') fonts.add(child.v); child?.kf?.forEach((k: any) => fonts.add(k.v)); }
              if (child?.expr) warnings.add('Expression-driven font families must also be supplied by the host app.');
            } else addFonts(child);
          }
        };
        addFonts(layer.d);
      }
      for (const effect of layer.fx || []) {
        // The compositor skips missing placeholders while retaining their source.
        // Keep that same behavior in the portable scene, including its keyframes.
        if (effect.missing === true) {
          warnings.add(`${layer.name}: unavailable effect “${effect.type}” remains inactive, as in the editor. Its editable data is preserved.`);
          continue;
        }
        getDef('effects', effect.type, layer.name);
      }
      for (const field of ['transitionIn', 'transitionOut']) if (layer[field]) getDef('transitions', layer[field].type, layer.name);
      if (layer.type === 'extension') {
        const def = getDef('layerTypes', layer.d.definition, layer.name);
        if (def?.renderer?.kind === 'mesh' && layer.d.data?.[def.renderer.assetField]) assets.add(layer.d.data[def.renderer.assetField]);
        if (def?.renderer?.kind === 'layer3d') for(const id of layer3DAssetIds(layer))assets.add(id);
        if (def?.renderer?.kind === 'scene3d') for(const id of sceneAssetIds(parseScene(layer.d.data?.scene))) assets.add(id);
      }
      if (layer.type === 'precomp') {
        const sub = comp.comps?.[layer.d.comp] || PM.proj.comps?.[layer.d.comp];
        if (!sub) errors.add(`${layer.name}: missing nested composition`); else visit(sub, depth + 1);
      }
      if (layer.type === 'video') warnings.add('Video playback depends on the browser supporting the source codec.');
    }
    active.delete(comp);
  };
  visit(PM.proj, 0);
  return { errors: [...errors], warnings: [...warnings], definitions, fonts, assets };
}

// Renditions are named by the original's content hash (a linked original's is its link's).
const renditionHash = (meta: any): string | null =>
  typeof meta.contentHash === 'string' ? meta.contentHash : meta.link?.hash ?? null;

/* The live asset still shows the frozen asset's original: the same content
   or link hash, or, for an asset restored without a hash, the same stored
   bytes (storage key and fingerprint). */
function showsFrozenOriginal(live: any, meta: any, native: boolean): boolean {
  if (!live || live.kind !== meta.kind) return false;
  const liveHash = typeof live.contentHash === 'string' ? live.contentHash : live.link?.hash;
  if (typeof liveHash === 'string') return liveHash === renditionHash(meta);
  return !native && !!meta.storageKey && typeof meta.fingerprint === 'string'
    && live.storageKey === meta.storageKey && live.fingerprint === meta.fingerprint;
}

/* The rendition the live asset shows, fetched before the export's first
   await: fetch resolves a blob: URL when it is called, so a relink or a
   project switch during the export cannot swap or revoke those bytes. */
function captureShownRendition(PM: any, id: string, meta: any): Promise<Blob | null> | null {
  const live = PM.assets?.get?.(id);
  const url = live?.url;
  if (typeof url !== 'string' || !url.startsWith('blob:') || !showsFrozenOriginal(live, meta, PM.MediaStore?.enabled === true)) return null;
  try { return fetch(url).then(response => response.blob()).catch(() => null); } catch { return null; }
}

/**
 * A browser plays what this viewer plays. With the new engine's media store,
 * an asset marked `rendition` stores its original (ProRes, TIFF, HEIF) and
 * keeps a browser rendition as generated media (section 21): the web export
 * takes the kept rendition, else the one the live asset shows when it still
 * renders the frozen original, else converts that original the way the
 * viewer does (`stillRendition`, `playbackProxy` in legacy/gl/raster.ts).
 */
async function browserRendition(PM: any, meta: any, project: any, shown: Promise<Blob | null> | null): Promise<Blob> {
  if (PM.EngineSourceMedia) return PM.EngineSourceMedia.rendition(project, meta.id, meta.kind === 'image' ? 'png' : 'webm');
  const store = PM.MediaStore;
  const hash = renditionHash(meta);
  const kept = hash ? await store?.rendition?.get?.(hash).catch(() => null) : null;
  if (kept) return kept;
  const live = await shown;
  if (live) return live;
  const label = meta.name || meta.id;
  if (store?.enabled === false) throw new Error(`“${label}” has no browser-playable copy yet. Open it in the viewer, then export again.`);
  const kind = meta.kind === 'video' ? 'video' : meta.kind === 'image' ? 'still' : null;
  if (!kind) throw new Error(`“${label}” cannot be converted for web playback.`);
  // Main picks the decoder by this name's extension, and prefers a link to the hash.
  const link = meta.storage === 'linked' && meta.link ? meta.link : null;
  const name = `original.${typeof meta.format === 'string' && meta.format ? meta.format : mediaExtension(meta.name)}`;
  const readOriginal = async (): Promise<Blob> => {
    const original = await store?.get?.(meta);
    if (!original) throw new Error(`The original media for “${label}” is missing. Reimport it before exporting.`);
    return original;
  };
  let blob: Blob;
  if (typeof store?.convert === 'function') {
    /* By reference first: main converts the stored object or the linked
       file. Only when it has neither are the original's bytes read here
       and uploaded, so main's own copy of a multi-gigabyte original is
       never drained into the page. */
    let result = hash || link ? await store.convert(kind, null, { hash, link, name }) : { ok: false, missing: true };
    if (!result.ok && result.missing) result = await store.convert(kind, await readOriginal(), { hash, link, name });
    if (!result.ok) throw new Error(result.error || `Could not prepare “${label}” for web playback.`);
    blob = await readProxyFile(result, kind === 'still' ? `${String(meta.name || 'image')}.png` : label, 0);
  } else if (kind === 'still') {
    // A remote host's browser: its bridge uploads the bytes to convert.
    const original = await readOriginal();
    blob = await convertStillImage(new File([original], name, { type: original.type }));
  } else {
    throw new Error(`“${label}” has no browser-playable copy yet. Open it in the viewer, then export again.`);
  }
  // With Use New Engine off the store is read-only, and an export writes nothing.
  if (hash && store?.enabled === true) await store.rendition?.put?.(hash, blob)?.catch?.(() => false);
  return blob;
}

const STORED_EXTENSIONS: Readonly<Record<string, string>> = {
  'video/webm': 'webm', 'video/mp4': 'mp4', 'video/quicktime': 'mov',
  'image/png': 'png', 'image/jpeg': 'jpg', 'image/webp': 'webp', 'image/gif': 'gif',
  'image/avif': 'avif', 'image/bmp': 'bmp', 'image/svg+xml': 'svg',
  'audio/wav': 'wav', 'audio/x-wav': 'wav', 'audio/mpeg': 'mp3',
  'audio/mp4': 'm4a', 'audio/ogg': 'ogg', 'audio/flac': 'flac',
};

export async function buildWebExport(PM: any) {
  // Freeze source and definitions before awaiting assets; user edits cannot mix revisions.
  const serialized = PM.serialize();
  const snapshot = typeof serialized === 'string' ? JSON.parse(serialized) : serialized;
  // Parsed JSON already belongs only to this export. Object-returning hosts
  // still need detached fields before filtering and asynchronous packaging.
  const project = cleanProject(snapshot.proj || snapshot, typeof serialized !== 'string');
  const inspection = inspectWebExport({ ...PM, proj: project });
  if (inspection.errors.length) throw new Error(inspection.errors.join('\n'));
  const files = new Map<string, Uint8Array>();
  const scene: WebScene = {
    format: 'powermove-web', version: 1, project, assets: {}, fonts: [],
    effects: structuredClone([...inspection.definitions.effects.values()]),
    transitions: structuredClone([...inspection.definitions.transitions.values()]),
    layerTypes: structuredClone([...inspection.definitions.layerTypes.values()]),
    warnings: [...inspection.warnings],
  };
  validateScene(scene);
  const put = (name: string, data: Uint8Array) => {
    files.set(name, data);
  };
  let index = 0;
  const metadata: Record<string, any> = {};
  const findAsset = (comp: any, id: string): any => {
    const own = comp.assets?.[id];
    if (own) return own;
    for (const sub of Object.values(comp.comps || {})) {
      const found = findAsset(sub, id);
      if (found) return found;
    }
  };
  const assetIds = [...inspection.assets];
  // Still before the first await: the renditions on screen now belong to this snapshot.
  const shown = new Map<string, Promise<Blob | null> | null>();
  for (const id of assetIds) {
    const meta = findAsset(project, id);
    if (meta?.rendition === true) shown.set(id, captureShownRendition(PM, id, meta));
  }
  // Overlap independent storage reads without starting an unbounded number of
  // large Blob conversions. Commit each batch in scene order so filenames and
  // ZIP entry order stay deterministic even when reads finish out of order.
  for (let start = 0; start < assetIds.length; start += 4) {
    const loaded = await Promise.allSettled(assetIds.slice(start, start + 4).map(async id => {
      const meta = findAsset(project, id);
      if (!meta) throw new Error(`Missing media metadata: ${id}`);
      const blob = meta.rendition === true ? await browserRendition(PM, meta, project, shown.get(id) ?? null) : await PM.MediaStore.get(meta);
      if (!blob) throw new Error(`The original media for “${meta.name || id}” is missing. Reimport it before exporting.`);
      return { id, meta, blob, data: new Uint8Array(await blob.arrayBuffer()) };
    }));
    // Drain active reads before reporting an error, so retrying cannot overlap
    // a failed export's remaining large Blob conversions.
    for (const result of loaded) {
      if (result.status === 'rejected') throw result.reason;
      const { id, meta, blob, data } = result.value;
      /* The stored bytes decide the extension: a converted animation or playback
         proxy keeps the name the user chose while holding WebM, and a static host
         serves these files by extension alone. */
      const extension = STORED_EXTENSIONS[String(blob.type || '').toLowerCase()]
        || /\.(png|jpg|jpeg|webp|gif|svg|mp4|webm|mov|wav|mp3|m4a|ogg|flac|obj|glb|gltf|cube)$/i.exec(meta.name || '')?.[1]?.toLowerCase()
        || 'bin';
      const location = `assets/media-${index++}.${extension}`;
      put(location, data); scene.assets[id] = location;
      metadata[id] = Object.fromEntries(['id', 'name', 'kind', 'format', 'w', 'h', 'dur', 'channels', 'sampleRate', 'type'].filter(k => meta[k] !== undefined).map(k => [k, meta[k]]));
      metadata[id].id = id;
    }
  }
  // All runtime media lookups use the root registry; strip disk paths and unused assets.
  const clearAssets = (comp: any) => { comp.assets = {}; Object.values(comp.comps).forEach(clearAssets); };
  clearAssets(project); project.assets = metadata;
  const faces: CSSFontFaceRule[] = [];
  const readRules = (rules: CSSRuleList) => {
    for (const rule of Array.from(rules)) {
      if (rule.type === 5) faces.push(rule as CSSFontFaceRule);
      else if ('cssRules' in rule) readRules((rule as CSSGroupingRule).cssRules);
    }
  };
  if (typeof document !== 'undefined') for (const sheet of Array.from(document.styleSheets)) { try { readRules(sheet.cssRules); } catch {} }
  const fontCandidates: Array<{ family: string; url: URL; weight: string; style: string; unicodeRange?: string }> = [];
  for (const family of inspection.fonts) {
    const matches = faces.filter(face => face.style.getPropertyValue('font-family').replace(/['"]/g, '').trim().toLowerCase() === family.toLowerCase());
    for (const face of matches) {
      const src = /url\(\s*['"]?([^)'"\s]+)['"]?\s*\)/.exec(face.style.getPropertyValue('src'))?.[1];
      if (!src) continue;
      try {
        fontCandidates.push({ family, url: new URL(src, face.parentStyleSheet?.href || document.baseURI),
          weight: face.style.getPropertyValue('font-weight') || '400', style: face.style.getPropertyValue('font-style') || 'normal',
          unicodeRange: face.style.getPropertyValue('unicode-range') || undefined });
      } catch { /* Host-provided font is recorded below. */ }
    }
  }
  const packagedFonts = new Set<string>();
  // Font families can contain many unicode subsets. Overlap fetch and body
  // reads, then insert in candidate order to preserve filenames and ZIP bytes.
  for (let start = 0; start < fontCandidates.length; start += 4) {
    const loaded = await Promise.all(fontCandidates.slice(start, start + 4).map(async candidate => {
      try {
        const response = await fetch(candidate.url);
        if (!response.ok) return null;
        return { candidate, bytes: new Uint8Array(await response.arrayBuffer()) };
      } catch { return null; /* Keep the existing host-font fallback. */ }
    }));
    for (const item of loaded) {
      if (!item) continue;
      const { candidate: { family, weight, style, unicodeRange }, bytes } = item;
      const path = `assets/font-${scene.fonts.length}.bin`;
      put(path, bytes);
      scene.fonts.push({ family, src: path, weight, style, unicodeRange });
      packagedFonts.add(family);
    }
  }
  for (const family of inspection.fonts) {
    if (!packagedFonts.has(family)) scene.warnings.push(`Font “${family}” is not bundled. Supply this font in your app to preserve text layout.`);
  }
  put('scene.json', utf8(JSON.stringify(scene, null, 2)));
  // The standalone runtime is only needed when packaging a web export. Keep
  // its source out of the editor's startup bundle; the snapshot above remains
  // fixed while this chunk loads, just as it does while media/fonts load.
  const { default: playerSource } = await import('virtual:powermove-player');
  put('player.js', utf8(playerSource));
  put('player.d.ts', utf8(playerTypes));
  put('index.html', utf8(previewHTML(project.name)));
  put('preview.js', utf8(previewScript));
  put('PowermoveAnimation.jsx', utf8(reactExample));
  put('README.md', utf8(readme(scene)));
  const handoff = agentHandoff(scene);
  put('handoff.json', utf8(JSON.stringify(handoff.manifest, null, 2)));
  put('AGENT_HANDOFF.md', utf8(handoff.instructions));
  put('HANDOFF_PROMPT.txt', utf8(handoff.prompt));
  put('package.json', utf8(JSON.stringify({ private: true, type: 'module' }, null, 2)));
  return { files, scene, bytes: zipFiles(files) };
}

function previewHTML(name: string) {
  const title = String(name || 'Powermove animation').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));
  return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${title}</title>
<style>body{margin:0;background:#111;color:#eee;font:15px system-ui;display:grid;place-content:center;min-height:100vh}main{width:min(90vw,960px)}canvas{width:100%;height:auto;background:repeating-conic-gradient(#222 0% 25%,#292929 0% 50%) 0/20px 20px}nav{display:flex;gap:16px;align-items:center;padding:20px 0}button{padding:10px 18px;border:0;border-radius:6px;cursor:pointer}input[type=range]{flex:1}#status{white-space:pre-wrap;color:#bbb}</style>
<main><h1>${title}</h1><canvas aria-label="${title}"></canvas><nav><button id="play" disabled>Play</button><button id="restart" disabled>Restart</button><input aria-label="Animation time" id="time" type="range" min="0" step="0.001" value="0" disabled><label><input id="loop" type="checkbox" checked>Loop</label></nav><p id="status" role="status">Loading animation…</p></main><script type="module" src="./preview.js"></script></html>`;
}
const previewScript = `import { createPlayer } from './player.js';
const status = document.querySelector('#status');
try {
  const scene = await (await fetch('./scene.json')).json();
  const player = await createPlayer({ canvas: document.querySelector('canvas'), scene, onError: error => status.textContent = error.message });
  const button = document.querySelector('#play'), slider = document.querySelector('#time');
  button.disabled = slider.disabled = document.querySelector('#restart').disabled = false;
  slider.max = player.duration;
  button.onclick = () => player.playing ? player.pause() : player.play();
  document.querySelector('#restart').onclick = async () => { await player.seek(0); player.play(); };
  slider.oninput = async () => { player.pause(); await player.seek(Number(slider.value)); };
  document.querySelector('#loop').onchange = event => player.loop = event.target.checked;
  const update = () => { slider.value = player.currentTime; button.textContent = player.playing ? 'Pause' : 'Play'; requestAnimationFrame(update); }; update();
  status.textContent = scene.warnings.join('\\n');
  window.addEventListener('pagehide', () => player.destroy(), { once: true });
} catch (error) { status.textContent = error.message; }
`;
const playerTypes = `export interface PlayerOptions {
  canvas: HTMLCanvasElement;
  scene: string | URL | object;
  baseURL?: string | URL;
  loop?: boolean;
  autoplay?: boolean;
  audio?: boolean;
  transparent?: boolean;
  onError?: (error: Error) => void;
}
export interface PowermovePlayer {
  readonly duration: number;
  readonly currentTime: number;
  readonly playing: boolean;
  readonly parameters: Record<string, { value: unknown; control: string; label?: string }>;
  loop: boolean;
  play(): void;
  pause(): void;
  seek(seconds: number): Promise<void>;
  setParameter(name: string, value: unknown): Promise<void>;
  setText(layerId: string, text: string): Promise<void>;
  destroy(): void;
}
export function createPlayer(options: PlayerOptions): Promise<PowermovePlayer>;
`;
const reactExample = `import { useEffect, useRef } from 'react';
import { createPlayer } from './player.js';

// Copy the exported folder into your app. Serve scene.json and assets together.
export function PowermoveAnimation({ src, onReady, onError, ...canvasProps }) {
  const host = useRef(null);
  const callbacks = useRef({ onReady, onError });
  callbacks.current = { onReady, onError };
  useEffect(() => {
    let cancelled = false, player;
    // A fresh canvas per effect also isolates React Strict Mode's setup/cleanup cycle.
    const canvas = document.createElement('canvas');
    canvas.style.width = '100%'; canvas.style.height = 'auto';
    host.current.append(canvas);
    createPlayer({ canvas, scene: src, onError: error => callbacks.current.onError?.(error) })
      .then(instance => {
        if (cancelled) instance.destroy();
        else { player = instance; callbacks.current.onReady?.(instance); }
      }).catch(error => { if (!cancelled) callbacks.current.onError?.(error); });
    return () => { cancelled = true; player?.destroy(); canvas.remove(); };
  }, [src]);
  return <div {...canvasProps} ref={host} />;
}
`;
function readme(scene: WebScene) {
  return `# Powermove web animation

This export contains editable scene data and a standalone WebGL2 renderer. No Electron, Powermove installation, account, CDN, or build step is required. The scene is rendered live, not flattened into video.

## Preview

Serve this folder over HTTP (for example: python3 -m http.server 8000), then open http://localhost:8000. Opening index.html as a file URL is not supported by browser module/fetch rules.

## Hand off to a coding agent

Attach this entire ZIP to your coding agent along with the destination app and desired placement/trigger. Paste HANDOFF_PROMPT.txt. AGENT_HANDOFF.md supplies implementation and verification instructions; handoff.json lists this composition's parameters, text-layer IDs, generated rendering dependencies, and compatibility notes.

## Embed

\`\`\`js
import { createPlayer } from './player.js';
const player = await createPlayer({
  canvas: document.querySelector('canvas'),
  scene: new URL('./scene.json', import.meta.url),
  loop: true,
  transparent: true,
  onError: console.error,
});
// Call play from a user interaction when the composition contains audio.
button.onclick = () => player.play();
player.pause();
await player.seek(1.5); // seconds
player.loop = false;
// Always clean up when unmounting:
// player.destroy();
\`\`\`

Canvas CSS controls display size; the drawing buffer follows that size and screen pixel density, within GPU limits. Omit transparent to preserve the composition background. autoplay defaults to false; loop defaults to true; audio defaults to true. Web audio may require a user gesture. Calls after destroy are invalid. seek clamps to the last frame. Playback stops on the last frame when loop is false.

Use await player.setParameter(name, value) for the composition parameters listed below. These drive existing param(name) expressions; changing an unreferenced parameter does not change the picture. await player.setText(layerId, text) overrides a root text layer's text animation for this player instance. Both operate on a private copy, leaving scene.json unchanged.

Parameters: ${literal(scene.project.params || {})}

Root text layers: ${literal(scene.project.layers.filter(l => l.type === 'text').map(l => ({ id: l.id, name: l.name })))}

## React

PowermoveAnimation.jsx is an integration component. Pass src as the served scene.json URL and use onReady(player) to wire interactions. It owns cleanup. This is a canvas component, not generated DOM elements for each layer.

## Compatibility

Requires a browser with WebGL2, ES modules, and the codecs used by the original media. Font files available from the editor's stylesheets are included; system fonts must be supplied separately. Ensure you have redistribution rights for your media and fonts. Editable JSON preserves keyframes, expressions, paths, masks, effects and nested compositions. Extension rendering definitions are bundled; editor panels and extension JavaScript are not.

${scene.warnings.length ? scene.warnings.map(w => '- ' + w).join('\n') : 'No export warnings.'}

Runtime and scene format are versioned together. Keep player.js, scene.json, and assets from the same export together.
`;
}
