/** Render Work Area keeps pixels in the native engine's leased GPU cache.
 * Playback continues through its ordinary video/audio clock and presenter. */
import type { DocumentSession } from './session';

export function installNativePreviewCache(PM: any, session: () => DocumentSession | null): () => void {
  const previous = PM.Preview;
  let generation = 0, preparing = false, count = 0, bytes = 0, active = false;
  const signature = () => `${PM.proj?.id}:${PM.animVersion?.() ?? 0}:${PM.proj?.work}:${PM.previewFps ?? PM.proj?.fps}`;
  const emit = () => { PM.bus.emit('preview'); PM.invalidate(); };
  const clear = () => { generation++; count = 0; bytes = 0; active = false; session()?.previewCache(0, false, true); emit(); };
  const play = () => { if (!count) return; PM.setTime(PM.proj.work?.[0] ?? 0, { raw: true }); active = true; PM.play(); emit(); };
  const preview = {
    clear, play, stop: () => { active = false; PM.pause(); emit(); },
    cancel: () => { generation++; session()?.previewCache(0, false, true); },
    get active() { return active && PM.playing; }, get preparing() { return preparing; },
    get count() { return count; }, get bytes() { return bytes; },
    async cache() {
      if (preparing) return;
      clear(); preparing = true; PM.pause();
      const own = generation, old = PM.time, key = signature();
      let current: DocumentSession | null = null;
      try {
        current = session();
        const size = current?.previewSize();
        if (!current || !size) throw new Error('Open the viewer before rendering the work area.');
        const fps = PM.previewFps || PM.proj.fps;
        const [start, end] = PM.proj.work ?? [0, PM.proj.dur];
        const frames = Math.max(1, Math.ceil((end - start) * fps));
        const limit = PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024;
        const required = frames * size.width * size.height * 8;
        if (!Number.isFinite(required) || required > limit || limit <= 0) throw new Error(limit <= 0
          ? 'Preview memory is off. Enable it in Settings → General.'
          : 'This preview exceeds your memory limit. Shorten the work area or lower preview resolution.');
        current.previewCache(limit, true, true);
        for (let i = 0; i < frames; i++) {
          if (generation !== own || signature() !== key) return;
          await current.cacheFrame(start + i / fps);
          count = i + 1; bytes = count * size.width * size.height * 8; emit();
        }
        // The lease may be smaller than the user's requested ceiling. Prove
        // the whole work area survived warming before offering cached play.
        const first = await current.cacheFrame(start);
        if (!first.quality.cached) throw new Error('This preview exceeds the engine’s available memory. Shorten the work area or lower preview resolution.');
        current.previewCache(limit, false);
        if (generation === own && signature() === key) play();
      } catch (e) { clear(); PM.toast(e instanceof Error ? e.message : String(e), 6000); }
      finally {
        if (!active) { current?.previewCache(0, false, true); count = 0; bytes = 0; PM.time = old; }
        preparing = false; emit();
      }
    },
  };
  PM.Preview = preview;
  const off = [PM.bus.on('project', clear), PM.bus.on('layers', clear), PM.bus.on('quality', clear)];
  return () => { clear(); for (const remove of off) if (typeof remove === 'function') remove(); if (PM.Preview === preview) PM.Preview = previous; };
}
