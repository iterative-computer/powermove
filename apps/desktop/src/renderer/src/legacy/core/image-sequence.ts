import { IMAGE_SEQUENCE_ACCEPT, orderedSequence, sequenceFrame, sequenceGaps, validSequenceFps } from '../../../../shared/image-sequence';
import type { PMRegistry } from '../registry';
import type { ImportProgress } from './import-progress';
import { bridge as hostBridge } from '../../kernel/bridge';

export const importedSequences = new WeakMap<File, { fps: number; frames: number }>();

export function chooseSequence(PM: PMRegistry, files: File[], required: boolean): Promise<{ files: File[]; fps: number | null } | null> {
  return new Promise(resolve => {
    const h = PM.h;
    let selected = files;
    let picker: HTMLInputElement | null = null;
    let closed = false;
    const fps = h('input.sequence-import-fps', { type: 'number', min: 1, max: 240, step: 'any', value: PM.curComp?.().fps || PM.proj.fps || 30, 'aria-label': 'Sequence frame rate' });
    const count = h('strong');
    const filenames = h('p.sequence-import-files');
    const duration = h('output.sequence-import-duration', { 'aria-label': 'Sequence duration' });
    const warningTitle = h('strong');
    const warningText = h('p');
    const cleanupPicker = () => { picker?.remove(); picker = null; };
    const reimport = h('button.btn.sequence-import-reimport', { type: 'button', onclick: () => {
      cleanupPicker();
      const input = document.createElement('input');
      input.type = 'file'; input.multiple = true; input.accept = IMAGE_SEQUENCE_ACCEPT;
      input.style.display = 'none'; picker = input;
      input.onchange = () => {
        if (!closed && input.files?.length) { selected = Array.from(input.files); update(); }
        cleanupPicker();
      };
      input.addEventListener('cancel', cleanupPicker, { once: true });
      body.appendChild(input); input.click();
    } }, 'Reimport…');
    const warning = h('div.sequence-import-warning', { role: 'status', 'aria-live': 'polite' },
      warningTitle, warningText, reimport);
    const update = () => {
      const validFps = validSequenceFps(Number(fps.value));
      count.textContent = `${selected.length} frames selected`;
      duration.textContent = validFps ? `${(selected.length / Number(fps.value)).toFixed(3)} s` : '—';
      fps.setAttribute('aria-invalid', String(!validFps));
      let ordered = selected;
      try {
        ordered = orderedSequence(selected);
        const gaps = sequenceGaps(ordered);
        const missing = gaps.reduce((count, gap) => count + gap.end - gap.start + 1, 0);
        const ranges = gaps.slice(0, 8).map(gap => gap.start === gap.end ? String(gap.start) : `${gap.start}–${gap.end}`).join(', ');
        warningTitle.textContent = missing
          ? `Missing frame ${missing === 1 ? 'number' : 'numbers'}: ${ranges}${gaps.length > 8 ? `, and ${gaps.length - 8} more gaps` : ''}`
          : '';
        warningText.textContent = 'You can still import. Available images will play consecutively, skipping the missing frames.';
      } catch (error) {
        warningTitle.textContent = 'Check your selection';
        warningText.textContent = error instanceof Error ? error.message : 'Choose numbered image frames';
      }
      filenames.textContent = `${ordered[0]?.name ?? ''} → ${ordered[ordered.length - 1]?.name ?? ''}`;
      filenames.title = filenames.textContent;
      warning.hidden = !warningTitle.textContent;
    };
    const body = h('div.sequence-import',
      h('p.sequence-import-intro', 'Turn your numbered images into one clip.'),
      h('div.sequence-import-source', h('span.sequence-import-icon', PM.icon('film')),
        h('div.sequence-import-source-text', count, filenames)),
      h('div.sequence-import-settings',
        h('label.sequence-import-rate', h('span', 'Frame rate'),
          h('span.sequence-import-input', fps, h('span', { 'aria-hidden': 'true' }, 'fps')),
          h('small', '1–240 fps')),
        h('div.sequence-import-timing', h('span', 'Clip duration'), duration)),
      warning);
    fps.addEventListener('input', update); update();
    PM.modal({ title: 'Import image sequence', width: 480, body,
      onClose: () => { closed = true; cleanupPicker(); resolve(null); },
      actions: [
        { label: 'Cancel' },
        ...(!required ? [{ label: 'Individual images', run: () => { resolve({ files: selected, fps: null }); } }] : []),
        { label: 'Import sequence', pri: true, run: () => {
          orderedSequence(selected);
          if (!validSequenceFps(Number(fps.value))) throw new Error('Frame rate must be between 1 and 240 fps');
          resolve({ files: selected, fps: Number(fps.value) });
        } },
      ],
    });
  });
}

/** Decode one source at a time to validate dimensions without retaining bitmaps. */
export async function convertImageSequence(files: File[], fps: number, assertCurrent: () => void, onProgress?: (progress: ImportProgress) => void): Promise<File> {
  const frames = orderedSequence(files);
  if (!validSequenceFps(fps)) throw new Error('Frame rate must be between 1 and 240 fps');
  const media = hostBridge()?.media;
  if (!media?.createImageSequence) throw new Error('Image sequence import requires the desktop app');
  let width = 0, height = 0;
  let checked = 0;
  onProgress?.({ label: `Checking frames · 0 of ${frames.length}`, completed: 0, total: frames.length });
  for (const file of frames) {
    assertCurrent();
    const bitmap = await createImageBitmap(file);
    try {
      if (!width) { width = bitmap.width; height = bitmap.height; }
      if (bitmap.width !== width || bitmap.height !== height) throw new Error(`Frame dimensions differ: ${file.name} · all frames must be ${width} × ${height}`);
    } finally { bitmap.close(); }
    onProgress?.({ label: `Checking frames · ${++checked} of ${frames.length}`, completed: checked, total: frames.length });
  }
  assertCurrent();
  onProgress?.({ label: 'Creating image sequence…' });
  const result = await media.createImageSequence(frames, fps, completed => {
    onProgress?.({ label: `Creating sequence · ${completed} of ${frames.length} frames`, completed, total: frames.length });
  });
  if (!result.ok) throw new Error(result.error);
  try {
    assertCurrent();
    const parts: ArrayBuffer[] = [];
    const chunkSize = 4 * 1024 * 1024;
    onProgress?.({ label: 'Loading image sequence…', completed: 0, total: result.size });
    for (let offset = 0; offset < result.size;) {
      assertCurrent();
      const chunk = await media.readPlaybackProxy(result.token, offset, Math.min(chunkSize, result.size - offset));
      if (!chunk.byteLength) throw new Error('The image sequence ended unexpectedly');
      parts.push(new Uint8Array(chunk).buffer);
      offset += chunk.byteLength;
      onProgress?.({ label: 'Loading image sequence…', completed: offset, total: result.size });
    }
    const frame = sequenceFrame(frames[0]!.name)!;
    const name = frame.prefix.replace(/[-_. ]+$/, '') || 'Image';
    const file = new File(parts, `${name} sequence.webm`, { type: result.type });
    importedSequences.set(file, { fps, frames: frames.length });
    return file;
  } finally { await media.releasePlaybackProxy(result.token).catch(() => undefined); }
}
