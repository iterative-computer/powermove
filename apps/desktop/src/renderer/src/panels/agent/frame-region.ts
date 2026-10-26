/* Screen region picks: the person drags a box anywhere over Powermove and the
   agent receives what was under it. A box on the composition sends the
   rendered frame with the box drawn on it, the time, the area in composition
   pixels and the layers there; elsewhere it sends that part of the window and
   the panels it covers. */
import { bridge } from '../../kernel/bridge';

export interface Rect { x: number; y: number; width: number; height: number }
/** A box in composition pixels. */
export type FrameRegion = Rect;

export interface ScreenPick {
  /** Client pixels; null when the person clicked instead of dragging. */
  rect: Rect | null;
  point: { x: number; y: number };
  /** The window as it looked before the pick layer appeared. */
  shot: ImageBitmap | null;
}

export interface FrameRegionCapture {
  file: File;
  /** Carried into the request's attachment mark so the agent knows what it is looking at. */
  note: string;
}

const MIN_DRAG = 6;
const CAPTURE_WIDTH = 1600;
const MAX_LAYERS = 12;

type Box = { left: number; top: number; width: number; height: number };
type Composition = { w: number; h: number };

/** Clamp a client-space drag to the composition frame and convert it to
    composition pixels. A click (or a box outside the frame) selects the whole frame. */
export function regionFromDrag(
  start: { x: number; y: number }, end: { x: number; y: number }, frame: Box, composition: Composition
): FrameRegion {
  const whole = { x: 0, y: 0, width: composition.w, height: composition.h };
  if (Math.abs(end.x - start.x) < MIN_DRAG && Math.abs(end.y - start.y) < MIN_DRAG) return whole;
  if (!(frame.width > 0 && frame.height > 0)) return whole;
  const clampX = (value: number) => Math.min(frame.left + frame.width, Math.max(frame.left, value));
  const clampY = (value: number) => Math.min(frame.top + frame.height, Math.max(frame.top, value));
  const x0 = clampX(Math.min(start.x, end.x)), x1 = clampX(Math.max(start.x, end.x));
  const y0 = clampY(Math.min(start.y, end.y)), y1 = clampY(Math.max(start.y, end.y));
  const sx = composition.w / frame.width, sy = composition.h / frame.height;
  const x = Math.round((x0 - frame.left) * sx), y = Math.round((y0 - frame.top) * sy);
  const width = Math.round((x1 - x0) * sx), height = Math.round((y1 - y0) * sy);
  return width < 1 || height < 1 ? whole : { x, y, width, height };
}

export function isWholeFrame(region: FrameRegion, composition: Composition): boolean {
  return region.x === 0 && region.y === 0 && region.width === composition.w && region.height === composition.h;
}

function overlapArea(a: Rect, b: Box): number {
  const width = Math.min(a.x + a.width, b.left + b.width) - Math.max(a.x, b.left);
  const height = Math.min(a.y + a.height, b.top + b.height) - Math.max(a.y, b.top);
  return width > 0 && height > 0 ? width * height : 0;
}

/** A box counts as "on the composition" when most of it lies inside the frame. */
export function onComposition(rect: Rect | null, point: { x: number; y: number }, frame: Box | null): boolean {
  if (!frame) return false;
  if (!rect) return point.x >= frame.left && point.x <= frame.left + frame.width && point.y >= frame.top && point.y <= frame.top + frame.height;
  return overlapArea(rect, frame) >= rect.width * rect.height * .8;
}

export function frameRegionNote(
  region: FrameRegion, composition: Composition & { fps: number; name?: string },
  time: number, timecode: string, layers: string[] = []
): string {
  const frame = Math.round(time * composition.fps);
  const at = `composition${composition.name ? ` "${composition.name}"` : ''} at ${timecode} (${time.toFixed(3)}s, frame ${frame} at ${composition.fps} fps)`;
  const where = isWholeFrame(region, composition)
    ? `whole frame of ${at}, ${composition.w}×${composition.h}`
    : `${at}; selected area x ${region.x}–${region.x + region.width}, y ${region.y}–${region.y + region.height} px of ${composition.w}×${composition.h}, outlined in orange`;
  return layers.length ? `${where}; layers there: ${layers.join(', ')}` : where;
}

/** Interface picks carry no playhead time: that only describes the composition. */
export function screenRegionNote(rect: Rect | null, viewport: { width: number; height: number }, panels: string[]): string {
  const area = rect
    ? `Powermove window area x ${rect.x}–${rect.x + rect.width}, y ${rect.y}–${rect.y + rect.height} of ${viewport.width}×${viewport.height} CSS px`
    : `whole Powermove window, ${viewport.width}×${viewport.height} CSS px`;
  const covering = panels.length ? `; covers panels: ${panels.join(', ')}` : '';
  return `${area}${covering}`;
}

function compositionFrame(PM: Record<string, any>): Box | null {
  const stage: HTMLElement | undefined = PM.Kernel?.services?.get?.('viewer')?.stage;
  const inner = stage?.isConnected ? stage.querySelector<HTMLElement>(':scope > #stage-inner') : null;
  if (!inner || !stage) return null;
  const frame = inner.getBoundingClientRect(), visible = stage.getBoundingClientRect();
  // Only the part of the frame the viewer actually shows can be picked.
  const left = Math.max(frame.left, visible.left), top = Math.max(frame.top, visible.top);
  const right = Math.min(frame.right, visible.right), bottom = Math.min(frame.bottom, visible.bottom);
  if (right - left < 4 || bottom - top < 4) return null;
  return { left: frame.left, top: frame.top, width: frame.width, height: frame.height };
}

/** Panels actually visible under the box, most covered first. Docks clip
    panels, so this samples what is on screen rather than panel rectangles. */
function panelsUnder(PM: Record<string, any>, rect: Rect | null): string[] {
  const area = rect ?? { x: 0, y: 0, width: window.innerWidth, height: window.innerHeight };
  const hits = new Map<string, number>();
  const steps = 10;
  for (let row = 0; row < steps; row++) for (let column = 0; column < steps; column++) {
    const x = area.x + area.width * (column + .5) / steps, y = area.y + area.height * (row + .5) / steps;
    const element = document.elementsFromPoint(x, y).find(node => !node.closest('.agent-frame-pick'));
    const id = element?.closest<HTMLElement>('.panel[data-panel]')?.dataset.panel;
    if (id) hits.set(id, (hits.get(id) ?? 0) + 1);
  }
  // A sliver along the edge of the box does not count as covering a panel.
  return [...hits].filter(([, count]) => count >= steps * steps * .05)
    .sort((a, b) => b[1] - a[1]).map(([id]) => String(PM.PANELS?.[id]?.title || id)).slice(0, 12);
}

/** Top-most first: layers active at `time` whose bounds overlap the region. */
function layersUnder(PM: Record<string, any>, region: FrameRegion, time: number): string[] {
  const viewer = PM.Kernel?.services?.get?.('viewer');
  const active: any[] = PM.ProjectIndex?.activeAt?.(time) ?? [];
  if (!viewer?.worldBounds) return [];
  // Composition layer order is the stacking order: index 0 is top-most.
  const stack = new Map<string, number>(((PM.curComp?.() || PM.proj).layers || []).map((layer: any, index: number) => [layer.id, index]));
  const box = { left: region.x, top: region.y, width: region.width, height: region.height };
  return active.map((layer, order) => {
    let bounds: any = null;
    try { bounds = viewer.worldBounds(layer, time); } catch { /* Layers without geometry are skipped. */ }
    const hit = bounds ? overlapArea({ x: bounds.x0, y: bounds.y0, width: bounds.x1 - bounds.x0, height: bounds.y1 - bounds.y0 }, box) : 0;
    return { layer, order: stack.get(layer.id) ?? stack.size + order, hit };
  }).filter(entry => entry.hit > 0)
    .sort((a, b) => a.order - b.order)
    .slice(0, MAX_LAYERS)
    .map(({ layer }) => `"${layer.name}" (${layer.type}, id ${layer.id})`);
}

function timeInfo(PM: Record<string, any>) {
  const composition = PM.curComp?.() || PM.proj;
  const time = Number(PM.time) || 0;
  const fps = Number(composition.fps || PM.proj.fps) || 30;
  return { composition, time, fps, timecode: PM.tc ? String(PM.tc(time, fps)) : `${time.toFixed(2)}s` };
}

/** Short description shown in the chip that follows the cursor. Matches the
    attachment the agent will receive. */
export function describePick(PM: Record<string, any>, rect: Rect | null, point: { x: number; y: number }) {
  const frame = compositionFrame(PM);
  if (onComposition(rect, point, frame)) {
    const { timecode } = timeInfo(PM);
    const region = regionFromDrag(
      rect ? { x: rect.x, y: rect.y } : point, rect ? { x: rect.x + rect.width, y: rect.y + rect.height } : point,
      frame!, { w: PM.proj.w, h: PM.proj.h });
    const whole = isWholeFrame(region, { w: PM.proj.w, h: PM.proj.h });
    return { label: `Frame ${timecode}${whole ? '' : ' selection'}`, detail: whole ? `${PM.proj.w}×${PM.proj.h}` : `${region.width}×${region.height} px` };
  }
  const panels = panelsUnder(PM, rect);
  const size = rect ? `${rect.width}×${rect.height}` : 'Whole window';
  return { label: rect ? 'Screen selection' : 'Screen', detail: panels.length ? `${size} · ${panels.slice(0, 2).join(', ')}${panels.length > 2 ? ` +${panels.length - 2}` : ''}` : size };
}

async function captureWindow(PM: Record<string, any>): Promise<ImageBitmap | null> {
  try {
    const bytes = await bridge()?.captureWindow?.();
    if (bytes) return await createImageBitmap(new Blob([bytes as BlobPart], { type: 'image/png' }));
  } catch { /* Fall back to the shell's capture bridge below. */ }
  return (await PM.WindowCapture?.request?.().catch(() => null)) ?? null;
}

const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));

/** Lets the person drag a box anywhere over the window. Resolves null on Escape. */
export async function pickScreenRegion(PM: Record<string, any>): Promise<ScreenPick | null> {
  PM.pause?.();
  // The agent popover is where the request is being written; keep it out of the shot.
  document.body.classList.add('agent-frame-capturing');
  await nextFrame(); await nextFrame();
  let shot: ImageBitmap | null = null;
  try { shot = await captureWindow(PM); }
  finally { document.body.classList.remove('agent-frame-capturing'); }
  document.body.classList.add('agent-frame-picking');

  return new Promise(resolve => {
    const layer = document.createElement('div');
    layer.className = 'agent-frame-pick';
    // A dialog role keeps the agent popover open while the pick is active.
    layer.setAttribute('role', 'dialog');
    layer.setAttribute('aria-label', 'Select part of the screen');
    const box = document.createElement('div');
    box.className = 'agent-frame-pick-box';
    box.hidden = true;
    const chip = document.createElement('div');
    chip.className = 'agent-frame-pick-chip';
    chip.setAttribute('aria-live', 'polite');
    const label = document.createElement('span'); label.className = 'agent-frame-pick-label';
    const detail = document.createElement('span'); detail.className = 'agent-frame-pick-detail';
    chip.append(label, detail);
    chip.hidden = true;
    layer.append(box, chip);
    document.body.append(layer);

    let start: { x: number; y: number } | null = null;
    const watch = new AbortController();
    const finish = (pick: ScreenPick | null) => {
      watch.abort();
      layer.remove();
      document.body.classList.remove('agent-frame-picking');
      if (!pick) shot?.close();
      resolve(pick);
    };
    const rectTo = (end: { x: number; y: number }): Rect | null => {
      if (!start || (Math.abs(end.x - start.x) < MIN_DRAG && Math.abs(end.y - start.y) < MIN_DRAG)) return null;
      const x0 = Math.max(0, Math.min(start.x, end.x)), x1 = Math.min(window.innerWidth, Math.max(start.x, end.x));
      const y0 = Math.max(0, Math.min(start.y, end.y)), y1 = Math.min(window.innerHeight, Math.max(start.y, end.y));
      return { x: Math.round(x0), y: Math.round(y0), width: Math.round(x1 - x0), height: Math.round(y1 - y0) };
    };
    const update = (point: { x: number; y: number }) => {
      const rect = rectTo(point);
      box.hidden = !rect;
      if (rect) Object.assign(box.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` });
      const info = describePick(PM, rect, start ?? point);
      label.textContent = info.label;
      detail.textContent = info.detail;
      chip.hidden = false;
      // Follow the cursor, flipping to stay inside the window.
      const bounds = chip.getBoundingClientRect();
      const left = point.x + 14 + bounds.width > window.innerWidth - 8 ? point.x - 14 - bounds.width : point.x + 14;
      const top = point.y + 18 + bounds.height > window.innerHeight - 8 ? point.y - 12 - bounds.height : point.y + 18;
      chip.style.transform = `translate(${Math.round(Math.max(8, left))}px, ${Math.round(Math.max(8, top))}px)`;
    };
    layer.addEventListener('pointerdown', event => {
      if (event.button !== 0) { finish(null); return; }
      event.preventDefault(); event.stopPropagation();
      start = { x: event.clientX, y: event.clientY };
      layer.setPointerCapture(event.pointerId);
      layer.classList.add('is-dragging');
      update(start);
    }, { signal: watch.signal });
    layer.addEventListener('pointermove', event => update({ x: event.clientX, y: event.clientY }), { signal: watch.signal });
    layer.addEventListener('pointerup', event => {
      if (!start) return;
      const point = { x: event.clientX, y: event.clientY };
      finish({ rect: rectTo(point), point: start, shot });
    }, { signal: watch.signal });
    layer.addEventListener('pointercancel', () => finish(null), { signal: watch.signal });
    window.addEventListener('keydown', event => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopImmediatePropagation();
      finish(null);
    }, { signal: watch.signal, capture: true });
    window.addEventListener('blur', () => { if (!start) finish(null); }, { signal: watch.signal });
  });
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('Could not read the captured frame.'));
    image.src = src;
  });
}

function encode(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => canvas.toBlob(
    result => result ? resolve(result) : reject(new Error('Could not encode the capture.')), 'image/jpeg', .9));
}

/** Renders the frame at the playhead and outlines the region on it. */
export async function captureFrameRegion(PM: Record<string, any>, region: FrameRegion): Promise<FrameRegionCapture> {
  const project = PM.proj;
  const { composition, time, fps, timecode } = timeInfo(PM);
  const width = Math.min(CAPTURE_WIDTH, project.w);
  const dataUrl: string = await (PM.Export.snapshotAsync?.(time, width) ?? PM.Export.snapshot(time, width));
  const image = await loadImage(dataUrl);
  const canvas = document.createElement('canvas');
  canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(image, 0, 0);
  const whole = isWholeFrame(region, project);
  if (!whole) {
    const scale = canvas.width / project.w;
    const x = region.x * scale, y = region.y * scale, w = region.width * scale, h = region.height * scale;
    // Dim what lies outside so the selected area reads first.
    ctx.fillStyle = 'rgb(0 0 0 / .45)';
    ctx.beginPath();
    ctx.rect(0, 0, canvas.width, canvas.height);
    ctx.rect(x, y, w, h);
    ctx.fill('evenodd');
    const line = Math.max(2, Math.round(canvas.width / 400));
    ctx.strokeStyle = '#ff6a1a';
    ctx.lineWidth = line;
    ctx.strokeRect(x - line / 2, y - line / 2, w + line, h + line);
  }
  const name = `Frame ${timecode}${whole ? '' : ' selection'}.jpg`;
  return {
    file: new File([await encode(canvas)], name, { type: 'image/jpeg' }),
    note: frameRegionNote(region, { w: project.w, h: project.h, fps, name: composition.name }, time, timecode, layersUnder(PM, region, time)),
  };
}

/** Crops the window shot taken before the pick. */
async function captureScreen(PM: Record<string, any>, pick: ScreenPick): Promise<FrameRegionCapture> {
  const shot = pick.shot;
  if (!shot) throw new Error('Could not capture the Powermove window.');
  const source = pick.rect ?? { x: 0, y: 0, width: window.innerWidth, height: window.innerHeight };
  const scaleX = shot.width / window.innerWidth, scaleY = shot.height / window.innerHeight;
  const sw = source.width * scaleX, sh = source.height * scaleY;
  const fit = Math.min(1, CAPTURE_WIDTH / Math.max(sw, sh));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(sw * fit)); canvas.height = Math.max(1, Math.round(sh * fit));
  canvas.getContext('2d', { alpha: false })!.drawImage(shot, source.x * scaleX, source.y * scaleY, sw, sh, 0, 0, canvas.width, canvas.height);
  const name = pick.rect ? 'Screen selection.jpg' : 'Screen.jpg';
  return {
    file: new File([await encode(canvas)], name, { type: 'image/jpeg' }),
    note: screenRegionNote(pick.rect, { width: window.innerWidth, height: window.innerHeight }, panelsUnder(PM, pick.rect)),
  };
}

/** Turns a finished pick into the attachment the agent receives. */
export async function capturePick(PM: Record<string, any>, pick: ScreenPick): Promise<FrameRegionCapture> {
  try {
    const frame = compositionFrame(PM);
    if (onComposition(pick.rect, pick.point, frame)) {
      const rect = pick.rect;
      const region = regionFromDrag(
        rect ? { x: rect.x, y: rect.y } : pick.point, rect ? { x: rect.x + rect.width, y: rect.y + rect.height } : pick.point,
        frame!, { w: PM.proj.w, h: PM.proj.h });
      return await captureFrameRegion(PM, region);
    }
    return await captureScreen(PM, pick);
  } finally {
    pick.shot?.close();
  }
}
