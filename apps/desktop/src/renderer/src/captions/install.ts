/*
 * Captions workflows in the editor: the `captions` service (cue selection,
 * inline-edit requests), Import / Export / Generate commands, menu entries,
 * Delete and Split on selected cues, and SRT/WebVTT files arriving through
 * media import or drops. Edits go through PM.Edit like every other change.
 */
import type { CaptionSelection, CaptionsService } from '../kernel/api';
import type { PMRegistry } from '../legacy/registry';
import { sourceTime } from '../legacy/core/retiming';
import { isProperty } from '../legacy/core/content-properties';
import { createSettingsSection } from '../legacy/ui/settings-section';
import { row as settingsRow, select as settingsSelect } from '../legacy/ui/project-settings';
import { mountSquircles, SQUIRCLE_SELECTOR } from '../settings/squircle';
import { ensureTranscriptionModel, transcribe } from '../transcription/client';
import { resolveMediaPath } from '../media/media-path';
import { exportCaptionsText } from './commands';
import { parseCaptions, type CaptionFormat } from './formats';
import { captionableLayer, captionsFromSpeech, type GenerateResult } from './generate';
import { cueIndexAt } from './model';
import { linearClip, type ClipTiming } from './time-map';
import CaptionsPanel from './CaptionsPanel.svelte';
import { registerSveltePanel } from '../panels/registerSveltePanel';
import { openPopoverMenu } from '../controls/popover-menu';

export const CAPTION_FILE = /\.(srt|vtt)$/i;
const OWNER = 'captions';

/** Composition timing of a clip, sampled only when its speed is animated or time-remapped. */
export function clipTiming(PM: any, layer: any): ClipTiming {
  const d = layer.d || {};
  const speed = d.speed;
  const varying = (isProperty(speed) && (speed.kf?.length || speed.expr)) || (d.timeRemap && isProperty(d.sourceTime));
  if (!varying) {
    const rate = Number(isProperty(speed) ? speed.v : speed ?? 1) || 1;
    const trim = Number(isProperty(d.trim) ? d.trim.v : d.trim) || 0;
    return linearClip(layer.from, layer.dur, trim, rate);
  }
  return { from: layer.from, dur: layer.dur, sourceAt: (time: number) => sourceTime(PM, layer, time) };
}

/** "Interview.en.srt" → { name: "Interview", language: "en" }. */
export function captionFileInfo(fileName: string): { name: string; language?: string } {
  const base = fileName.replace(CAPTION_FILE, '');
  const match = /^(.*)\.([a-z]{2,3}(?:-[A-Za-z0-9]{2,8})?)$/.exec(base);
  return match && match[1] ? { name: match[1], language: match[2] } : { name: base || 'Captions' };
}

export function installCaptions(PM: PMRegistry): void {
  const selection: CaptionSelection = { layerId: null, cues: [] };
  const listeners = new Set<() => void>();
  const editListeners = new Set<(request: { layerId: string; cueId: string }) => boolean | void>();
  const toast = (text: string, options: Record<string, unknown> = {}) => PM.toast?.(text, (options.ms as number) ?? 2600, options);

  const notify = () => {
    for (const listener of [...listeners]) { try { listener(); } catch (error) { console.error('[captions]', error); } }
    PM.invalidate?.('timeline');
  };
  const captionLayers = () => (PM.proj?.layers || []).filter((layer: any) => layer.type === 'captions');
  const layer = (id: string | null | undefined) => (id ? PM.L?.(id) : null) as any;

  function select(layerId: string | null, cues: string[]) {
    const target = layer(layerId);
    const next = target?.type === 'captions' ? { layerId: target.id, cues: [...new Set(cues)] } : { layerId: null, cues: [] };
    const same = next.layerId === selection.layerId && next.cues.length === selection.cues.length && next.cues.every((id, i) => id === selection.cues[i]);
    selection.layerId = next.layerId; selection.cues = next.cues;
    // Cues are selected on their own: the layer becomes the only selected one.
    if (target && !onlySelected(target.id)) PM.selectLayers?.([target.id]);
    if (!same) notify();
  }
  const onlySelected = (id: string) => { const layers = PM.sel?.layers || []; return layers.length === 1 && layers[0] === id; };

  /* Selecting any other layer (alone or alongside), deleting the layer, or an
     Undo that removes the cues ends the cue selection, so Delete and Split
     never act on cues the user is no longer looking at. */
  const prune = () => {
    if (!selection.layerId) return;
    const target = layer(selection.layerId);
    if (!target || target.type !== 'captions' || !onlySelected(target.id)) {
      selection.layerId = null; selection.cues = []; notify(); return;
    }
    const ids = new Set((target.d?.cues || []).map((cue: any) => cue.id));
    const kept = selection.cues.filter(id => ids.has(id));
    if (kept.length !== selection.cues.length) { selection.cues = kept; notify(); }
  };
  PM.bus?.on?.('sel', prune);
  PM.bus?.on?.('layers', prune);
  PM.bus?.on?.('project', prune);

  const apply = (command: any, label: string, origin = 'command') => PM.Edit.apply(command, { label, origin });

  function deleteSelectedCues(): unknown {
    const target = layer(selection.layerId);
    if (!target || !selection.cues.length || !onlySelected(target.id)) return false;
    const ids = [...selection.cues];
    const result = apply({ type: 'edit_captions', target: target.id, op: 'delete', ids }, ids.length === 1 ? 'Delete caption' : 'Delete captions', 'timeline');
    if (result?.ok) { selection.cues = []; notify(); }
    return result;
  }

  /** Split the selected cue under the playhead (Premiere's Add Edit on a caption track). */
  function splitAtPlayhead(): unknown {
    const target = layer(selection.layerId);
    if (!target || !selection.cues.length) return false;
    const local = PM.time - target.from;
    const index = cueIndexAt(target.d.cues, local);
    const cue = index >= 0 ? target.d.cues[index] : null;
    if (!cue || !selection.cues.includes(cue.id)) return false;
    const result = apply({ type: 'edit_captions', target: target.id, op: 'split', id: cue.id, at: PM.time }, 'Split caption', 'timeline');
    if (result?.ok) select(target.id, [cue.id, result.data?.results?.[0]?.data?.tailId].filter(Boolean));
    return result;
  }

  /** A new cue at the playhead on the selected (or first) captions layer. */
  function addCaption(): unknown {
    let target = layer(selection.layerId) || (PM.selLayers?.() || []).find((item: any) => item.type === 'captions') || captionLayers()[0];
    const at = PM.time;
    const fps = Number(PM.proj?.fps) || 30;
    if (!target) {
      const created = apply({ type: 'add_captions', from: 0, cues: [] }, 'New captions', 'command');
      if (!created?.ok) return created;
      target = layer(created.data?.results?.[0]?.data?.id);
    }
    if (!target) return false;
    const local = at - target.from;
    const cues = target.d.cues || [];
    if (cueIndexAt(cues, local) >= 0) { toast('A caption is already showing here'); return false; }
    const next = cues.find((cue: any) => cue.start > local);
    const end = Math.min(at + 2, next ? next.start + target.from : Infinity);
    if (end - at < 1 / fps) { toast('There is no room for a caption here'); return false; }
    const before = new Set(cues.map((cue: any) => cue.id));
    const result = apply({ type: 'edit_captions', target: target.id, op: 'insert', cues: [{ start: at, end, text: 'Caption' }] }, 'Add caption', 'timeline');
    if (!result?.ok) return result;
    const added = layer(target.id).d.cues.find((cue: any) => !before.has(cue.id));
    if (added) { select(target.id, [added.id]); editCue(target.id, added.id); }
    return result;
  }

  function editCue(layerId: string, cueId: string) {
    for (const listener of [...editListeners].reverse()) if (listener({ layerId, cueId }) === true) return;
  }

  async function readFile(file: File): Promise<string> {
    const buffer = await file.arrayBuffer();
    // UTF-8 first; a file that is not valid UTF-8 is almost always Windows-1252.
    try { return new TextDecoder('utf-8', { fatal: true }).decode(buffer); }
    catch { return new TextDecoder('windows-1252').decode(buffer); }
  }

  function pickFile(): Promise<File | null> {
    return new Promise(resolve => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.srt,.vtt,text/vtt,application/x-subrip';
      input.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none';
      const done = (file: File | null) => { input.remove(); resolve(file); };
      input.onchange = () => done(input.files?.[0] ?? null);
      input.addEventListener('cancel', () => done(null), { once: true });
      document.body.appendChild(input);
      input.click();
    });
  }

  async function importFile(file?: File, options: { at?: number } = {}): Promise<string | null> {
    const chosen = file ?? await pickFile();
    if (!chosen) return null;
    const project = PM.proj;
    const text = await readFile(chosen);
    if (PM.proj !== project) return null;
    const parsed = parseCaptions(text, 'auto', chosen.name);
    if (!parsed.cues.length) { toast(`No captions found in ${chosen.name}`, { error: true }); return null; }
    const info = captionFileInfo(chosen.name);
    const from = Math.max(0, Number(options.at) || 0);
    const result = apply({
      type: 'add_captions', name: info.name, text, format: parsed.format, offset: from, from,
      ...(info.language ? { language: info.language } : {})
    }, 'Import captions', 'import');
    if (!result?.ok) { toast(result?.message || 'Could not import captions', { error: true }); return null; }
    const id = result.data?.results?.[0]?.data?.id ?? null;
    toast(`Imported ${parsed.cues.length} ${parsed.cues.length === 1 ? 'caption' : 'captions'}${parsed.skipped ? ` · ${parsed.skipped} unreadable skipped` : ''}`);
    return id;
  }

  function exportTarget(layerId?: string) {
    const explicit = layer(layerId);
    if (explicit?.type === 'captions') return explicit;
    const selected = (PM.selLayers?.() || []).find((item: any) => item.type === 'captions');
    return selected || captionLayers()[0] || null;
  }

  async function saveSidecar(target: any, format: CaptionFormat, range?: { from: number; to: number; rebase?: number }): Promise<boolean> {
    const text = exportCaptionsText(target, format, range);
    const base = `${PM.proj?.name || 'Captions'}${captionLayers().length > 1 ? ` - ${target.name}` : ''}`.replace(/[\\/:]/g, '-');
    const result = await PM.download(new Blob([text], { type: format === 'vtt' ? 'text/vtt' : 'application/x-subrip' }), `${base}.${format}`);
    if (result?.cancelled) return false;
    toast(`Exported ${format === 'vtt' ? 'WebVTT' : 'SRT'} captions`);
    return true;
  }

  /** Export Captions…: a Settings-style sheet with layer, format and range. */
  function exportDialog(layerId?: string): Promise<boolean> {
    const layers = captionLayers();
    if (!layers.length) { toast('There are no captions to export'); return Promise.resolve(false); }
    const target = exportTarget(layerId);
    const state = { layerId: target.id as string, format: 'srt' as CaptionFormat, range: 'all' as 'all' | 'work' };
    const body = document.createElement('div');
    body.className = 'export-form sg-column captions-export';
    body.addEventListener('keydown', event => event.stopPropagation());
    const section = createSettingsSection('Captions');
    const field = (label: string, options: Array<{ v: string; label: string }>, get: () => string, set: (value: string) => void) => {
      const element = settingsSelect(label);
      for (const option of options) { const node = document.createElement('option'); node.value = option.v; node.textContent = option.label; element.append(node); }
      element.value = get();
      element.onchange = () => set(element.value);
      return element;
    };
    if (layers.length > 1) {
      section.body.append(settingsRow('Captions layer', 'Which captions to write.',
        field('Captions layer', layers.map((item: any) => ({ v: item.id, label: item.name })), () => state.layerId, value => { state.layerId = value; })).element);
    }
    section.body.append(settingsRow('Format', 'SubRip plays almost everywhere; WebVTT is the format for the web.',
      field('Format', [{ v: 'srt', label: 'SubRip (.srt)' }, { v: 'vtt', label: 'WebVTT (.vtt)' }], () => state.format, value => { state.format = value as CaptionFormat; })).element);
    const work = PM.proj?.work;
    if (work && work[1] > work[0] && (work[0] > 0 || work[1] < PM.proj.dur)) {
      section.body.append(settingsRow('Range', 'Times in the file start from the beginning of the range.',
        field('Range', [{ v: 'all', label: 'Whole composition' }, { v: 'work', label: 'Work area' }], () => state.range, value => { state.range = value as 'all' | 'work'; })).element);
    }
    body.append(section.element);
    return new Promise(resolve => {
      let settled = false;
      const finish = (value: boolean) => { if (!settled) { settled = true; resolve(value); } };
      let unmount: (() => void) | undefined;
      const modal = PM.modal({
        title: 'Export Captions', body, width: 520,
        onClose: () => { unmount?.(); finish(false); },
        actions: [{ label: 'Cancel' }, { label: 'Export', pri: true, run: () => {
          const chosen = layer(state.layerId);
          if (!chosen) return;
          const range = state.range === 'work' ? { from: work[0], to: work[1] } : undefined;
          window.setTimeout(() => { void saveSidecar(chosen, state.format, range && { ...range, rebase: range.from }).then(finish, () => finish(false)); }, 0);
        } }]
      });
      modal?.el?.classList?.add('export-modal');
      if (modal?.el) unmount = mountSquircles(modal.el, `${SQUIRCLE_SELECTOR}, .btn`);
    });
  }

  async function exportFile(layerId?: string, format?: 'srt' | 'vtt'): Promise<boolean> {
    if (!format) return exportDialog(layerId);
    const target = exportTarget(layerId);
    if (!target) { toast('There are no captions to export'); return false; }
    return saveSidecar(target, format);
  }

  let generating: AbortController | null = null;
  async function generate(layerIds?: string[], options: {
    origin?: string;
    /** Applies the add_captions command instead of PM.Edit (the agent's transaction). */
    apply?: (command: any) => any;
    interactive?: boolean;
    /** Aborts the generation from outside (an agent run that ended). */
    signal?: AbortSignal;
    onProgress?: (fraction: number) => void;
  } = {}): Promise<{ ok: boolean; layerId?: string; message?: string; status?: GenerateResult['status'] }> {
    const ids = layerIds?.length ? layerIds : (PM.sel?.layers || []);
    const layers = ids.map((id: string) => layer(id)).filter(captionableLayer);
    if (!layers.length) {
      const message = 'Select an audio or video clip to caption.';
      if (options.interactive !== false) toast(message);
      return { ok: false, message };
    }
    if (generating) { const message = 'Captions are already being generated.'; toast(message); return { ok: false, message }; }
    const project = PM.proj;
    const controller = new AbortController();
    generating = controller;
    const stop = () => controller.abort();
    if (options.signal?.aborted) controller.abort();
    else options.signal?.addEventListener('abort', stop, { once: true });
    const key = 'captions-generate';
    /* The sticky progress toast appears once a model is ready (not behind the
       download sheet) and every exit replaces it with a completed one. */
    let shown = false, settled = false;
    const progress = (fraction: number) => {
      if (settled || controller.signal.aborted) return;
      shown = true;
      options.onProgress?.(fraction);
      toast('Generating captions', {
        key, sticky: true, progress: fraction, icon: 'captions',
        action: { label: 'Cancel', run: () => controller.abort() }
      });
    };
    const finish = (text: string, extra: Record<string, unknown> = {}) => {
      settled = true;
      toast(text, { key, completed: true, ...extra });
    };
    try {
      const result = await captionsFromSpeech(layers, {
        ensureModel: async reason => {
          const ready = await ensureTranscriptionModel(reason);
          if (ready && PM.proj === project) progress(0);
          return ready;
        },
        resolvePath: resolveMediaPath,
        transcribe,
        clipTiming: (item: any) => clipTiming(PM, item),
        progress: fraction => progress(fraction),
        signal: controller.signal
      });
      if (PM.proj !== project) {
        const message = 'The project changed before captions were ready.';
        if (shown) finish('Caption generation stopped: the project changed');
        return { ok: false, message };
      }
      if (result.status === 'model-missing') {
        finish('Captions need a transcription model');
        return { ok: false, status: 'model-missing', message: 'No transcription model is installed. Download one in Settings › Transcription.' };
      }
      if (result.status === 'empty') {
        finish('No speech was found in the selected clips');
        return { ok: false, status: 'empty', message: 'No speech was found in the selected clips.' };
      }
      if (controller.signal.aborted) throw new DOMException('Caption generation was cancelled', 'AbortError');
      const name = layers.length === 1 ? `${layers[0].name} Captions` : 'Captions';
      const command = { type: 'add_captions', name, from: 0, cues: result.cues, ...(result.language ? { language: result.language } : {}) };
      const applied = options.apply ? options.apply(command) : apply(command, 'Generate captions', options.origin ?? 'command');
      if (!applied?.ok) throw new Error(applied?.message || 'Could not add the captions');
      finish(`Added ${result.cues.length} ${result.cues.length === 1 ? 'caption' : 'captions'}`);
      return { ok: true, status: 'ok', layerId: applied.data?.results?.[0]?.data?.id };
    } catch (error: any) {
      const cancelled = error?.name === 'AbortError' || controller.signal.aborted;
      const message = cancelled ? 'Caption generation was cancelled.' : error?.message || 'Could not generate captions.';
      finish(cancelled ? 'Caption generation cancelled' : message, cancelled ? {} : { error: true });
      return { ok: false, message };
    } finally {
      options.signal?.removeEventListener('abort', stop);
      if (shown && !settled) finish('Caption generation stopped');
      if (generating === controller) generating = null;
    }
  }

  const service: CaptionsService = {
    selection: () => ({ layerId: selection.layerId, cues: [...selection.cues] }),
    select,
    onChange(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    editCue,
    onEditRequest(listener) { editListeners.add(listener); return () => { editListeners.delete(listener); }; },
    importFile,
    exportFile,
    generate: (ids) => generate(ids)
  };

  PM.Captions = { ...service, deleteSelectedCues, splitAtPlayhead, addCaption, generate, exportText: exportCaptionsText, isCaptionFile: (file: File) => CAPTION_FILE.test(file?.name || '') };
  PM.Kernel?.services?.register?.('captions', service);

  /* Captions panel: the cue list. Its one header action opens the caption
     workflows from a button, so it is a popover menu, not a native one. */
  if (typeof PM.registerPanel === 'function') {
    registerSveltePanel(PM, 'captions', {
      title: 'Captions', size: 240, component: CaptionsPanel,
      header: (hdr) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'iconbtn panel-action';
        button.title = 'Caption actions';
        button.setAttribute('aria-label', 'Caption actions');
        button.setAttribute('aria-haspopup', 'menu');
        const glyph = PM.icon?.('plus');
        if (glyph instanceof Node) button.appendChild(glyph);
        button.addEventListener('click', () => openPopoverMenu({
          anchor: button, label: 'Caption actions',
          items: [
            { label: 'Add Caption at Playhead', run: () => { addCaption(); } },
            '-',
            { label: 'Import Captions…', run: () => { void importFile(); } },
            { label: 'Generate Captions', run: () => { void generate(); } },
            { label: 'Export Captions…', run: () => { void exportFile(); } }
          ]
        }));
        const options = hdr.querySelector('.panel-options');
        if (options) hdr.insertBefore(button, options); else hdr.appendChild(button);
      }
    });
  }

  const kernel = PM.Kernel;
  if (kernel?.commands?.register) {
    const command = (id: string, label: string, run: () => unknown, extra: Record<string, unknown> = {}) =>
      kernel.commands.register(OWNER, { id, label, category: 'Captions', run, ...extra });
    command('importCaptions', 'Import Captions…', () => { void importFile(); });
    command('exportCaptions', 'Export Captions…', () => { void exportFile(); });
    command('generateCaptions', 'Generate Captions', () => { void generate(); });
    command('addCaption', 'Add Caption at Playhead', () => addCaption());
  }
  if (kernel?.contributeMenu) {
    kernel.contributeMenu(OWNER, 'layer:context', (ctx: any) => {
      const target = layer(ctx?.layerId);
      if (captionableLayer(target)) return ['-', { label: 'Generate Captions', icon: 'captions', run: () => void generate([target.id]) }];
      if (target?.type === 'captions') return ['-', { label: 'Add Caption at Playhead', icon: 'captions', run: () => addCaption() }, { label: 'Export Captions…', run: () => void exportFile(target.id) }];
      return [];
    });
    kernel.contributeMenu(OWNER, 'timeline:context', () => [{ label: 'Import Captions…', icon: 'captions', run: () => void importFile() }]);
  }
}
