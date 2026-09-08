import playerSource from 'virtual:powermove-player';
import { validateScene, type WebScene } from './scene';
import { zipFiles } from './archive';
import { agentHandoff } from './agent-handoff';

const utf8 = (text: string) => new TextEncoder().encode(text);
const literal = (value: unknown) => JSON.stringify(value).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');

/** Only render source crosses the boundary; workspace, notes and edit history stay in the app. */
function cleanProject(comp: any): any {
  const fields = ['id', 'name', 'w', 'h', 'fps', 'dur', 'bg', 'backgroundFill', 'layers', 'assets', 'params', 'shutter'];
  const copy = Object.fromEntries(fields.filter(key => comp[key] !== undefined).map(key => [key, structuredClone(comp[key])]));
  copy.comps = Object.fromEntries(Object.entries(comp.comps || {}).map(([id, sub]) => [id, cleanProject(sub)]));
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

export async function buildWebExport(PM: any) {
  // Freeze source and definitions before awaiting assets; user edits cannot mix revisions.
  const serialized = PM.serialize();
  const snapshot = typeof serialized === 'string' ? JSON.parse(serialized) : serialized;
  const project = cleanProject(snapshot.proj || snapshot);
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
  let total = 0;
  const put = (name: string, data: Uint8Array) => {
    total += data.length;
    if (total > 256 * 1024 * 1024) throw new Error('Web exports are limited to 256 MB');
    files.set(name, data);
  };
  let index = 0;
  const metadata: Record<string, any> = {};
  const findAsset = (comp: any, id: string): any => comp.assets?.[id] || Object.values(comp.comps || {}).map(sub => findAsset(sub, id)).find(Boolean);
  for (const id of inspection.assets) {
    const meta = findAsset(project, id);
    if (!meta) throw new Error(`Missing media metadata: ${id}`);
    const blob = await PM.MediaStore.get(meta);
    if (!blob) throw new Error(`The original media for “${meta.name || id}” is missing. Reimport it before exporting.`);
    if (total + blob.size > 256 * 1024 * 1024) throw new Error('Web exports are limited to 256 MB');
    const extension = /\.(png|jpg|jpeg|webp|gif|svg|mp4|webm|mov|wav|mp3|m4a|ogg|flac|obj)$/i.exec(meta.name || '')?.[1]?.toLowerCase() || 'bin';
    const location = `assets/media-${index++}.${extension}`;
    put(location, new Uint8Array(await blob.arrayBuffer())); scene.assets[id] = location;
    metadata[id] = Object.fromEntries(['id', 'name', 'kind', 'format', 'w', 'h', 'dur', 'channels', 'sampleRate', 'type'].filter(k => meta[k] !== undefined).map(k => [k, meta[k]]));
    metadata[id].id = id;
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
  for (const family of inspection.fonts) {
    const matches = faces.filter(face => face.style.getPropertyValue('font-family').replace(/['"]/g, '').trim().toLowerCase() === family.toLowerCase());
    let packaged = false;
    for (const face of matches) {
      const src = /url\(\s*['"]?([^)'"\s]+)['"]?\s*\)/.exec(face.style.getPropertyValue('src'))?.[1];
      if (!src) continue;
      try {
        const response = await fetch(new URL(src, face.parentStyleSheet?.href || document.baseURI));
        if (!response.ok) continue;
        const bytes = new Uint8Array(await response.arrayBuffer());
        const path = `assets/font-${scene.fonts.length}.bin`;
        put(path, bytes);
        scene.fonts.push({ family, src: path, weight: face.style.getPropertyValue('font-weight') || '400', style: face.style.getPropertyValue('font-style') || 'normal', unicodeRange: face.style.getPropertyValue('unicode-range') || undefined });
        packaged = true;
      } catch { /* Host-provided font is recorded below. */ }
    }
    if (!packaged) scene.warnings.push(`Font “${family}” is not bundled. Supply this font in your app to preserve text layout.`);
  }
  put('scene.json', utf8(JSON.stringify(scene, null, 2)));
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

Canvas CSS controls display size; its drawing resolution is the composition size. Omit transparent to preserve the composition background. autoplay defaults to false; loop defaults to true; audio defaults to true. Web audio may require a user gesture. Calls after destroy are invalid. seek clamps to the last frame. Playback stops on the last frame when loop is false.

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
