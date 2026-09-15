import { IMAGE_SEQUENCE_ACCEPT, orderedSequence, sequenceFrame, sequenceGaps, validSequenceFps } from '../../../../shared/image-sequence';
import type { PMRegistry } from '../registry';

export const importedSequences = new WeakMap<File, { fps: number; frames: number }>();

export function chooseSequence(PM: PMRegistry, files: File[], required: boolean): Promise<{ files: File[]; fps: number | null } | null> {
  return new Promise(resolve => {
    const h = PM.h;
    let selected = files;
    let picker: HTMLInputElement | null = null;
    let closed = false;
    const fps = h('input', { type: 'number', min: 1, max: 240, step: 'any', value: PM.curComp?.().fps || PM.proj.fps || 30, 'aria-label': 'Sequence frame rate' });
    const duration = h('p');
    const warning = h('p', { role: 'status', 'aria-live': 'polite' });
    const cleanupPicker = () => { picker?.remove(); picker = null; };
    const reimport = h('button.btn', { type: 'button', style: { justifySelf: 'start' }, onclick: () => {
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
    const update = () => {
      duration.textContent = validSequenceFps(Number(fps.value))
        ? `${selected.length} frames · ${(selected.length / Number(fps.value)).toFixed(3)} seconds`
        : 'Enter a frame rate between 1 and 240 fps.';
      try {
        const gaps = sequenceGaps(selected);
        const missing = gaps.reduce((count, gap) => count + gap.end - gap.start + 1, 0);
        const ranges = gaps.slice(0, 8).map(gap => gap.start === gap.end ? String(gap.start) : `${gap.start}–${gap.end}`).join(', ');
        warning.textContent = missing
          ? `Missing frame ${missing === 1 ? 'number' : 'numbers'}: ${ranges}${gaps.length > 8 ? `, and ${gaps.length - 8} more gaps` : ''}. Importing skips missing frames and plays the available images consecutively. Reimport to choose files again, or cancel.`
          : '';
      } catch (error) { warning.textContent = error instanceof Error ? error.message : 'Choose numbered image frames'; }
      warning.hidden = !warning.textContent;
      reimport.hidden = !warning.textContent;
    };
    const body = h('div', { style: { display: 'grid', gap: '12px' } }, h('p', 'Import numbered images as one clip. Frames are ordered by number and transparency is preserved.'),
      h('label', { style: { display: 'grid', gap: '6px' } }, h('span', 'Frame rate (fps)'), fps), duration, warning, reimport);
    fps.addEventListener('input', update); update();
    PM.modal({ title: 'Import image sequence', width: 460, body,
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
export async function convertImageSequence(files: File[], fps: number, assertCurrent: () => void): Promise<File> {
  const frames = orderedSequence(files);
  if (!validSequenceFps(fps)) throw new Error('Frame rate must be between 1 and 240 fps');
  const media = window.powermove?.media;
  if (!media?.createImageSequence) throw new Error('Image sequence import requires the desktop app');
  let width = 0, height = 0;
  for (const file of frames) {
    assertCurrent();
    const bitmap = await createImageBitmap(file);
    try {
      if (!width) { width = bitmap.width; height = bitmap.height; }
      if (bitmap.width !== width || bitmap.height !== height) throw new Error(`Frame dimensions differ: ${file.name} · all frames must be ${width} × ${height}`);
    } finally { bitmap.close(); }
  }
  assertCurrent();
  const result = await media.createImageSequence(frames, fps);
  if (!result.ok) throw new Error(result.error);
  try {
    assertCurrent();
    const parts: ArrayBuffer[] = [];
    const chunkSize = 4 * 1024 * 1024;
    for (let offset = 0; offset < result.size;) {
      assertCurrent();
      const chunk = await media.readPlaybackProxy(result.token, offset, Math.min(chunkSize, result.size - offset));
      if (!chunk.byteLength) throw new Error('The image sequence ended unexpectedly');
      parts.push(new Uint8Array(chunk).buffer);
      offset += chunk.byteLength;
    }
    const frame = sequenceFrame(frames[0]!.name)!;
    const name = frame.prefix.replace(/[-_. ]+$/, '') || 'Image';
    const file = new File(parts, `${name} sequence.webm`, { type: result.type });
    importedSequences.set(file, { fps, frames: frames.length });
    return file;
  } finally { await media.releasePlaybackProxy(result.token).catch(() => undefined); }
}
