import type { PMRegistry } from '../registry';

export function previewResolution(value: unknown): 'auto' | '1' | '0.5' | '0.25' | null {
  const mode = String(value);
  return mode === 'auto' || mode === '1' || mode === '0.5' || mode === '0.25' ? mode : null;
}

/** The requested mode is independent of Auto's current rendering scale. */
export function installPreviewResolution(PM: PMRegistry): void {
  const saved = previewResolution(PM.store?.get?.('previewResolution', null));
  if (saved) {
    PM.previewResolution = saved;
    PM.perf.auto = saved === 'auto';
    PM.quality = saved === 'auto' ? 1 : Number(saved);
  }
  PM.setPreviewResolution = (value: unknown) => {
    const mode = previewResolution(value);
    if (!mode) return;
    PM.previewResolution = mode;
    PM.perf.auto = mode === 'auto';
    PM.quality = mode === 'auto' ? 1 : Number(mode);
    PM.store?.set?.('previewResolution', mode);
    // Retire frames rendered at the old resolution and resize immediately,
    // including while paused. This is a view preference, not a project edit.
    PM.bus.emit('quality');
    PM.invalidate('render');
    PM.invalidate('status');
  };
}
