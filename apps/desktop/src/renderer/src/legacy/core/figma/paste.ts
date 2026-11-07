import type { PMRegistry } from '../../registry';
import { isFieldTarget, selectableTextRoot } from '../../../kernel/keychord';
import type { FigmaSelection } from './clipboard';
import { clipboardDoc } from './adapter';
import { applyFigmaPlan, figmaMatches, materializeFigma } from './convert';
import { figmaImportSummary } from './summary';
import {
  clipboardFallbacks,
  convertFigma,
  selectionBounds,
  unsupportedReasons,
} from '../../../../../shared/figma/convert';
import {
  bounds,
  FIGMA_CLIPBOARD_PREFIX,
  MAX_PAYLOAD,
  validateFigmaDoc,
  type FigmaDoc,
  type FigmaNode,
} from '../../../../../shared/figma/doc';
const FIGMA_NOTICE = 'figma-import';
const pending = new WeakMap<PMRegistry, Promise<any>>();
type PasteSource = { html: string; image?: File; project: any; generation: number; compId: string; at: number };
export { FIGMA_CLIPBOARD_PREFIX } from '../../../../../shared/figma/doc';
export function pasteFigma(PM: PMRegistry, event: ClipboardEvent): boolean {
  if (
    event.defaultPrevented ||
    event.composedPath().some((t) => isFieldTarget(t) || selectableTextRoot(t as EventTarget))
  )
    return false;
  const data = event.clipboardData,
    html = data?.getData('text/html') ?? '',
    text = data?.getData('text/plain') ?? '';
  if (!html.includes('(figma)') && !text.startsWith(FIGMA_CLIPBOARD_PREFIX)) return false;
  event.preventDefault();
  const files = [
    ...Array.from(data?.files ?? []),
    ...Array.from(data?.items ?? [])
      .filter((i) => i.kind === 'file')
      .map((i) => i.getAsFile())
      .filter((f): f is File => !!f),
  ];
  const source = {
    html,
    image: files.find((f) => f.type === 'image/png'),
    project: PM.proj,
    generation: PM.projGeneration,
    compId: PM.proj.compId,
    at: PM.time,
  };
  const run = async () => {
    try {
      if (text.startsWith(FIGMA_CLIPBOARD_PREFIX)) {
        if (text.length > MAX_PAYLOAD) throw new Error('Figma selection is too large');
        await receiveFigma(PM, validateFigmaDoc(JSON.parse(text.slice(FIGMA_CLIPBOARD_PREFIX.length))), source);
      } else await runFigmaPaste(PM, source);
    } catch (e) {
      PM.toast(e instanceof Error ? e.message : 'Could not import Figma layers', 6000, { error: true });
    }
  };
  const task = (pending.get(PM) ?? Promise.resolve()).then(run, run);
  pending.set(PM, task);
  void task.finally(() => {
    if (pending.get(PM) === task) pending.delete(PM);
  });
  return true;
}
const current = (PM: PMRegistry, source: PasteSource) =>
  PM.proj === source.project && PM.projGeneration === source.generation && PM.proj.compId === source.compId;
export async function runFigmaPaste(
  PM: PMRegistry,
  source: PasteSource,
  read?: (html: string) => Promise<FigmaSelection>,
): Promise<void> {
  try {
    PM.dismissToast?.(FIGMA_NOTICE);
    let doc = read ? clipboardDoc(await read(source.html)) : await (await import('./decode')).readFigmaDoc(source.html);
    if (!current(PM, source)) return;
    const prepared = clipboardFallbacks(doc);
    doc = { ...doc, nodes: prepared.nodes };
    const image = source.image ?? inlinePng(source.html);
    if (prepared.whole) {
      const b = selectionBounds(doc.nodes);
      doc.nodes = [
        {
          id: doc.nodes.map((n) => n.id).join('+'),
          fileKey: 'clipboard',
          type: 'GROUP',
          name: 'Figma selection',
          absoluteTransform: [1, 0, 0, 1, b.x, b.y],
          w: b.w,
          h: b.h,
          fills: [],
          strokes: [],
          effects: [],
          children: [],
          unsupported: ['overlapping clipboard preview'],
        },
      ];
    }
    const fallbacks: FigmaNode[] = [];
    const collect = (n: FigmaNode) => {
      if (unsupportedReasons(n).length) fallbacks.push(n);
      else n.children.forEach(collect);
    };
    doc.nodes.forEach(collect);
    if (fallbacks.length) {
      if (!image)
        throw new Error('Figma did not include a preview. Use the Send to Powermove plugin for these layers.');
      const bitmap = await createImageBitmap(image);
      try {
        const box = selectionBounds(doc.nodes);
        for (const node of fallbacks) {
          const b = bounds(node);
          const c = document.createElement('canvas');
          c.width = Math.max(1, Math.ceil((b.w * bitmap.width) / box.w));
          c.height = Math.max(1, Math.ceil((b.h * bitmap.height) / box.h));
          if (c.width * c.height > 32e6) throw new Error('Figma preview is too large');
          const ctx = c.getContext('2d')!;
          ctx.drawImage(
            bitmap,
            ((b.x - box.x) * bitmap.width) / box.w,
            ((b.y - box.y) * bitmap.height) / box.h,
            (b.w * bitmap.width) / box.w,
            (b.h * bitmap.height) / box.h,
            0,
            0,
            c.width,
            c.height,
          );
          node.raster = { png: c.toDataURL('image/png').split(',')[1]!, bounds: b };
        }
      } finally {
        bitmap.close();
      }
    }
    await receiveFigma(PM, doc, source);
  } catch (e) {
    if (current(PM, source))
      PM.toast(e instanceof Error ? e.message : 'Could not paste Figma layers', 7000, { error: true });
  }
}
export async function receiveFigma(
  PM: PMRegistry,
  doc: FigmaDoc,
  source: PasteSource = {
    html: '',
    project: PM.proj,
    generation: PM.projGeneration,
    compId: PM.proj.compId,
    at: PM.time,
  },
): Promise<{ layers: number; flattened: number; missing: string[]; warnings?: import('../../../../../shared/figma/convert').Issue[] }> {
  if (!current(PM, source) || PM.ProjectsScreen?.isOpen) throw new Error('Open a composition in Powermove first');
  PM.dismissToast?.(FIGMA_NOTICE);
  const neutral = convertFigma(doc, PM.proj),
    plan = materializeFigma(PM, neutral, source.at);
  let update = false;
  if (figmaMatches(PM, plan).length) {
    const choice = await new Promise<boolean | null>((resolve) => {
      let done = false;
      const settle = (v: boolean | null) => {
        if (!done) {
          done = true;
          resolve(v);
        }
      };
      PM.modal({
        title: 'Update from Figma',
        body: 'This selection contains layers already in this composition.',
        width: 420,
        actions: [
          { label: 'Add as New Copy', run: () => settle(false) },
          { label: 'Update Existing Layers', pri: true, run: () => settle(true) },
        ],
        onClose: () => settle(null),
      });
    });
    if (choice === null) throw new Error('Figma update cancelled');
    update = choice;
  }
  if (!current(PM, source)) throw new Error('The active composition changed. Send again.');
  const existingAssets = new Set(Object.keys(source.project.assets)),
    importedAssets: Record<string, any> = {};
  let missing: string[] = [];
  try {
    for (let i = 0; i < neutral.commands.length; i++) {
      const command = neutral.commands[i]!;
      if (command.layerType !== 'image') continue;
      if (!command.raster && !command.imageHash) throw new Error(`${command.name} needs a preview. Use Send to Powermove in Figma.`);
      const png = Uint8Array.from(atob(command.imageHash ? doc.images[command.imageHash]! : command.raster!.png), (c) => c.charCodeAt(0));
      const mime = png[0]===137&&png[1]===80&&png[2]===78&&png[3]===71 ? 'image/png'
        : png[0]===255&&png[1]===216 ? 'image/jpeg' : png[0]===71&&png[1]===73&&png[2]===70 ? 'image/gif'
        : png[0]===82&&png[1]===73&&png[8]===87&&png[9]===69 ? 'image/webp' : undefined;
      if (!mime) throw new Error('Invalid Figma PNG');
      const result = (await PM.assets.importBatch([new File([png], `${command.name}.${mime.split('/')[1]}`, { type: mime })]))[0];
      if (!result || result.status === 'failed') throw new Error('Could not import Figma preview');
      plan.layers[i].d.asset = result.asset.id;
      if (!existingAssets.has(result.asset.id))
        importedAssets[result.asset.id] = source.project.assets[result.asset.id];
      if (!current(PM, source)) throw new Error('The active composition changed. Send again.');
    }
    if (!current(PM, source)) throw new Error('The active composition changed. Send again.');
    // Media is durable on disk already; include new project asset records in the
    // same history transaction as the layers, so undo restores the whole import.
    for (const id of Object.keys(importedAssets)) delete source.project.assets[id];
    ({ missing } = applyFigmaPlan(PM, plan, update, importedAssets));
  } catch (error) {
    for (const id of Object.keys(importedAssets)) delete source.project.assets[id];
    throw error;
  }
  const flattened = plan.issues.length;
  const warnings = plan.warnings ?? [];
  const summary = figmaImportSummary({
    update,
    layers: plan.layers.length,
    issues: plan.issues,
    missing,
    warnings,
    clipboard: doc.source === 'clipboard',
  });
  // Warnings name source refs; show the layers they became.
  const warnedLayers = summary.warned
    .map((ref) => plan.layers[neutral.commands.findIndex((command) => command.ref === ref)]?.id)
    .filter((id): id is string => typeof id === 'string');
  PM.toast(summary.message, warnings.length ? 12000 : 8000, {
    key: FIGMA_NOTICE,
    kind: 'status',
    ...(doc.source === 'clipboard' && flattened
      ? { action: { label: 'Connect Figma', run: () => PM.openFigmaConnect?.() } }
      : warnedLayers.length
        ? {
            action: {
              label: warnedLayers.length === 1 ? 'Show Layer' : 'Show Layers',
              run: () => {
                const present = warnedLayers.filter((id) => PM.L?.(id));
                if (present.length) PM.selectLayers?.(present);
              },
            },
          }
        : {}),
  });
  return { layers: plan.layers.length, flattened, missing, warnings };
}
function inlinePng(html: string): File | undefined {
  const data = /<img\b[^>]*\bsrc\s*=\s*["']data:image\/png;base64,([A-Za-z0-9+/=]+)["']/i.exec(html)?.[1];
  if (!data || data.length > MAX_PAYLOAD) return;
  try {
    return new File([Uint8Array.from(atob(data), (c) => c.charCodeAt(0))], 'Figma.png', { type: 'image/png' });
  } catch {
    return;
  }
}
