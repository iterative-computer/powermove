import type { PMRegistry } from '../../registry';
import { isFieldTarget, selectableTextRoot } from '../../../kernel/keychord';
import type { FigmaSelection } from './clipboard';
import { applyFigmaPlan, planFigmaImport, type FigmaPlan } from './convert';

const pending = new WeakMap<PMRegistry, Promise<void>>();
type PasteSource = { html: string; image?: File; project: any; generation: number; compId: string; at: number };

/** Claim Figma paste synchronously, ahead of media and the app-local clipboard. */
export function pasteFigma(PM: PMRegistry, event: ClipboardEvent): boolean {
  if (event.defaultPrevented || event.composedPath().some(target => isFieldTarget(target)
    || selectableTextRoot(target as EventTarget))) return false;
  const transfer = event.clipboardData;
  const html = transfer?.getData?.('text/html') ?? '';
  if (!html.includes('(figma)')) return false;
  event.preventDefault();
  const files = [...Array.from(transfer?.files ?? []), ...Array.from(transfer?.items ?? [])
    .filter(item => item.kind === 'file').map(item => item.getAsFile()).filter((file): file is File => !!file)];
  const source: PasteSource = { html, image: files.find(file => file.type === 'image/png'),
    project: PM.proj, generation: PM.projGeneration, compId: PM.proj.compId, at: PM.time };
  const run = () => runFigmaPaste(PM, source);
  const task = (pending.get(PM) ?? Promise.resolve()).then(run, run);
  pending.set(PM, task);
  void task.finally(() => { if (pending.get(PM) === task) pending.delete(PM); });
  return true;
}

export async function runFigmaPaste(PM: PMRegistry, source: PasteSource,
  read: (html: string) => Promise<FigmaSelection> = async html => (await import('./decode')).readFigmaSelection(html)): Promise<void> {
  const current = () => PM.proj === source.project && PM.projGeneration === source.generation && PM.proj.compId === source.compId;
  if (!current()) return;
  let plan: FigmaPlan | undefined;
  try {
    let problem: string | undefined;
    try { plan = planFigmaImport(PM, await read(source.html), source.at); }
    catch (error) { problem = error instanceof Error ? error.message : 'Could not read the copied Figma layers.'; }
    if (!current()) return;
    if (problem || plan?.issues.length) {
      // Native sheets limit detail length. Keep the explanation and the loss
      // of editability visible even when layer names are very long.
      const reasons = problem?.slice(0, 600) ?? plan!.issues.slice(0, 6)
        .map(issue => `“${issue.name.slice(0, 80)}”: ${issue.reason.slice(0, 120)}`).join('\n');
      const image = source.image ?? inlinePng(source.html);
      if (!image) {
        await PM.confirm({ message: 'These Figma layers need an image fallback',
          detail: `${reasons}\n\nFigma did not include a preview image. Use Copy as PNG in Figma, then paste again. The selection will paste as one image; its individual parts will not be editable.`, confirmLabel: 'OK' });
        return;
      }
      const proceed = await PM.confirm({ message: 'Paste Figma selection as an image?',
        detail: `${reasons}\n\nThese features cannot stay editable yet. The entire selection will use an image fallback. You can animate the image, but its individual parts will not be editable.`, confirmLabel: 'Paste as Image' });
      if (!proceed || !current()) return;
      // Reuse durable media import, but create the layer ourselves so source
      // dimensions, placement and the single undo step stay under our control.
      const results = await PM.assets.importBatch([new File([image], 'Figma selection.png', { type: 'image/png' })]);
      if (!current()) return;
      const result = results[0];
      if (!result || result.status === 'failed') throw new Error('Could not import the Figma preview image.');
      const command = PM.commandForAsset(result.asset.id, source.at);
      if (!command) throw new Error('Could not create the Figma image layer.');
      command.name = 'Figma selection (image fallback)';
      if (plan && result.asset.w > 0 && result.asset.h > 0) {
        const scale = Math.min(plan.width / result.asset.w, plan.height / result.asset.h);
        command.content.w = result.asset.w * scale; command.content.h = result.asset.h * scale;
      }
      const applied = PM.Edit.apply(command, { label: 'Paste Figma image', origin: 'import' });
      if (!applied.ok) throw new Error(applied.message);
      PM.toast('Pasted Figma selection as an image fallback', 5000);
    } else if (plan) {
      applyFigmaPlan(PM, plan);
      PM.toast(`Pasted ${plan.layers.length} Figma ${plan.layers.length === 1 ? 'layer' : 'layers'}`);
    }
  } catch (error) {
    if (current()) PM.toast(error instanceof Error ? error.message : 'Could not paste Figma layers.', 6000, { error: true });
  }
}

function inlinePng(html: string): File | undefined {
  // Only inline PNG previews are accepted. Never fetch URLs from clipboard HTML.
  const data = /<img\b[^>]*\bsrc\s*=\s*["']data:image\/png;base64,([A-Za-z0-9+/=]+)["']/i.exec(html)?.[1];
  if (!data || data.length > 32 * 1024 * 1024) return;
  try { return new File([Uint8Array.from(atob(data), char => char.charCodeAt(0))], 'Figma selection.png', { type: 'image/png' }); }
  catch { return; }
}
