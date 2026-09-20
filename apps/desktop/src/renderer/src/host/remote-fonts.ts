/*
 * Fonts on a remote host. The picker lists this browser's fonts plus the
 * host's; faces the host has that this browser lacks are registered from the
 * host so text renders here; and when the project uses a family the host does
 * not have, this browser sends that family's files up, once, so every other
 * device and every later session has it too.
 */
import { WEB, WEB_UPLOAD_CHUNK_BYTES } from '../../../shared/wire';

export interface FontLink {
  invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>;
}

interface HostFace { family: string; postscriptName: string; fullName: string; style: string; file: string; type: string }
interface LocalFace { family: string; fullName: string; postscriptName: string; style: string; blob(): Promise<Blob> }

type Registry = {
  proj?: { layers?: Array<{ type?: string; d?: { font?: unknown } }>; comps?: Record<string, { layers?: Array<{ type?: string; d?: { font?: unknown } }> }> };
  bus: { on(event: string, listener: (...args: any[]) => void): () => void; emit(event: string, ...args: unknown[]): void };
  Fonts?: { system: string[]; setSystemFamilies(values: string[]): void };
  toast?(text: string, ms?: number, options?: { error?: boolean }): void;
  rasterClear?(): void;
  invalidate?(what?: string): void;
};

const norm = (value: unknown): string => String(value ?? '').trim().replace(/^['"]|['"]$/g, '').toLocaleLowerCase();

function descriptors(style: string): FontFaceDescriptors {
  const s = style.toLowerCase();
  const weight = /thin|hairline/.test(s) ? '100' : /extra ?light|ultra ?light/.test(s) ? '200' : /light/.test(s) ? '300'
    : /medium/.test(s) ? '500' : /semi ?bold|demi ?bold/.test(s) ? '600' : /extra ?bold|ultra ?bold/.test(s) ? '800'
    : /black|heavy/.test(s) ? '900' : /bold/.test(s) ? '700' : '400';
  return { weight, style: /italic|oblique/.test(s) ? 'italic' : 'normal' };
}

/** Text layers' families across the composition and its nested comps. */
function familiesInUse(PM: Registry): string[] {
  const out = new Set<string>();
  const scan = (layers?: Array<{ type?: string; d?: { font?: unknown } }>) => {
    for (const layer of layers ?? []) if (layer.type === 'text' && typeof layer.d?.font === 'string' && layer.d.font.trim()) out.add(layer.d.font.trim());
  };
  scan(PM.proj?.layers);
  for (const comp of Object.values(PM.proj?.comps ?? {})) scan(comp.layers);
  return [...out];
}

async function localFaces(family: string): Promise<LocalFace[]> {
  const query = (globalThis as { queryLocalFonts?: () => Promise<LocalFace[]> }).queryLocalFonts;
  if (typeof query !== 'function') return [];
  try {
    const all = await query();
    const key = norm(family);
    return all.filter((face) => norm(face.family) === key);
  } catch { return []; }
}

async function uploadFace(link: FontLink, face: LocalFace): Promise<void> {
  const blob = await face.blob();
  const id = await link.invoke<string>(WEB.uploadBegin, { name: 'font', size: blob.size, kind: 'blob' });
  try {
    for (let offset = 0; offset < blob.size; offset += WEB_UPLOAD_CHUNK_BYTES) {
      const data = new Uint8Array(await blob.slice(offset, offset + WEB_UPLOAD_CHUNK_BYTES).arrayBuffer());
      await link.invoke(WEB.uploadChunk, { id, offset, data });
    }
    const hostPath = await link.invoke<string>(WEB.uploadFinish, id);
    await link.invoke(WEB.fontCommit, { family: face.family, postscriptName: face.postscriptName, fullName: face.fullName ?? '', style: face.style ?? '', path: hostPath });
  } catch (error) {
    await link.invoke(WEB.uploadAbort, id).catch(() => undefined);
    throw error;
  }
}

export function attachRemoteFonts(link: FontLink, PM: Registry): () => void {
  let hostFamilies: string[] = [];
  let hostFaces: HostFace[] = [];
  const registered = new Set<string>();
  const sent = new Set<string>();
  const failed = new Set<string>();
  let syncTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;

  /* the picker sees both machines' fonts */
  const fonts = PM.Fonts;
  const original = fonts?.setSystemFamilies.bind(fonts);
  let lastLocal: string[] = fonts?.system ?? [];
  if (fonts && original) {
    fonts.setSystemFamilies = (values: string[]) => {
      lastLocal = Array.isArray(values) ? values : [];
      original([...new Set([...lastLocal, ...hostFamilies])]);
    };
  }
  const republish = () => { if (fonts && original) original([...new Set([...lastLocal, ...hostFamilies])]); };

  /* faces the host has that this browser lacks render from the host */
  const registerHostFaces = async () => {
    const local = new Set(lastLocal.map(norm));
    let added = false;
    for (const face of hostFaces) {
      if (registered.has(face.postscriptName) || local.has(norm(face.family))) continue;
      registered.add(face.postscriptName);
      try {
        const fontFace = new FontFace(face.family, `url(/__powermove/fonts/file?ps=${encodeURIComponent(face.postscriptName)})`, descriptors(face.style));
        document.fonts.add(fontFace);
        await fontFace.load();
        added = true;
      } catch (error) { console.warn('[remote-fonts] could not load', face.postscriptName, error); }
    }
    if (added) { PM.rasterClear?.(); PM.invalidate?.(); }
  };

  const refresh = async () => {
    try {
      const list = await link.invoke<{ families: string[]; faces: HostFace[] }>(WEB.fontsList);
      hostFamilies = list.families;
      hostFaces = list.faces;
      republish();
      await registerHostFaces();
    } catch (error) { console.warn('[remote-fonts] list failed', error); }
  };

  /* families this project uses that the host lacks go up, once */
  const sync = async () => {
    syncTimer = null;
    if (disposed) return;
    const wanted = familiesInUse(PM).filter((family) => !sent.has(norm(family)) && !failed.has(norm(family)));
    if (!wanted.length) return;
    let present: string[] = [];
    try { present = await link.invoke<string[]>(WEB.fontsHas, wanted); } catch { return; }
    const have = new Set(present.map(norm));
    for (const family of wanted) {
      const key = norm(family);
      if (have.has(key)) { sent.add(key); continue; }
      const faces = await localFaces(family);
      if (!faces.length) { failed.add(key); continue; }
      sent.add(key);
      try {
        for (const face of faces) await uploadFace(link, face);
        PM.toast?.(`Sent ${family} to the host`, 2500, { error: false });
        await refresh();
      } catch (error) {
        sent.delete(key); failed.add(key);
        PM.toast?.(`Could not send ${family} to the host: ${error instanceof Error ? error.message : String(error)}`, 5000);
      }
    }
  };
  const schedule = () => { if (!syncTimer) syncTimer = setTimeout(() => { void sync(); }, 800); };

  const offs = ['layers', 'project', 'fonts'].map((event) => PM.bus.on(event, schedule));
  void refresh().then(schedule);
  const onFocus = () => { void refresh(); };
  window.addEventListener('focus', onFocus);
  return () => {
    disposed = true;
    for (const off of offs) off();
    window.removeEventListener('focus', onFocus);
    if (syncTimer) clearTimeout(syncTimer);
    if (fonts && original) fonts.setSystemFamilies = original;
  };
}
