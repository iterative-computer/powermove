import { resolveStudioQuality, type StudioQuality } from './studio-quality';

/* What each render quality means to the person choosing it. One source for
   the composition's Render Quality section, the Export dialog and the viewer's
   Refine indicator, so the three always use the same names and estimates. */

export type StudioQualityChoice = StudioQuality['preset'];

export const STUDIO_QUALITY_CHOICES: ReadonlyArray<{ id: StudioQualityChoice; label: string; summary: string }> = Object.freeze([
  { id: 'draft', label: 'Draft', summary: 'Fast and grainy. For blocking out motion and timing.' },
  { id: 'preview', label: 'Preview', summary: 'Clean enough to review. Quick drafts to share.' },
  { id: 'production', label: 'Production', summary: 'Clean frames with light focused through glass. The default.' },
  { id: 'final', label: 'Final', summary: 'The cleanest frames, for hero shots and stills. Slowest.' },
  { id: 'custom', label: 'Custom', summary: 'Your own samples, bounces and time per frame.' },
]);

/** The advanced settings Custom reveals, in After Effects' Title Case. */
export const STUDIO_QUALITY_FIELDS: ReadonlyArray<{
  key: Exclude<keyof StudioQuality, 'preset'>; label: string; kind: 'num' | 'toggle';
  min?: number; max?: number; step?: number; unit?: string; precision?: number; help: string;
}> = Object.freeze([
  { key: 'maxSamples', label: 'Max Samples', kind: 'num', min: 1, max: 4096, step: 1, help: 'The most light paths traced per pixel in each frame.' },
  { key: 'minSamples', label: 'Min Samples', kind: 'num', min: 1, max: 4096, step: 1, help: 'Paths traced before Studio checks whether a pixel is clean enough to stop.' },
  { key: 'noiseThreshold', label: 'Noise Threshold', kind: 'num', min: 0, max: 1, step: 0.005, precision: 3, help: 'Stop a pixel once its noise is below this. Lower is cleaner and slower; 0 always traces Max Samples.' },
  { key: 'timeLimitSeconds', label: 'Time per Frame', kind: 'num', min: 0, max: 3600, step: 0.5, unit: 's', precision: 1, help: 'The longest Studio traces one frame. 0 means no limit.' },
  { key: 'maxBounces', label: 'Max Bounces', kind: 'num', min: 1, max: 32, step: 1, help: 'How many times light can bounce. More shows deeper glass and interiors.' },
  { key: 'caustics', label: 'Caustics', kind: 'toggle', help: 'Light focused through glass and off mirrors onto other surfaces.' },
  { key: 'denoise', label: 'Denoise', kind: 'toggle', help: 'Remove the remaining grain after tracing.' },
  { key: 'motionBlurSamples', label: 'Blur Samples', kind: 'num', min: 1, max: 64, step: 1, help: 'Moments in the shutter blended for 3D motion blur. 1 turns 3D motion blur off.' },
]);

export function studioQualityLabel(choice: unknown): string {
  return STUDIO_QUALITY_CHOICES.find(c => c.id === choice)?.label ?? 'Production';
}

const duration = (seconds: number): string => {
  if (seconds < 1) return `${Math.max(0.1, Math.round(seconds * 10) / 10)} s`;
  if (seconds < 90) return `${Math.round(seconds * 10) / 10} s`;
  const minutes = seconds / 60;
  if (minutes < 90) return `${Math.round(minutes)} min`;
  return `${Math.round(minutes / 6) / 10} h`;
};

/** Plain description of a quality setting. `frames` adds the whole-export
 * estimate. Studio stops each frame at the earlier of its time limit and its
 * sample count, so the time limit is an upper bound, not a promise: simple
 * scenes finish sooner, and denoising adds a little on top. */
export function describeStudioQuality(raw: unknown, frames?: number): { label: string; summary: string; perFrame: string; total: string | null } {
  let quality: StudioQuality;
  try { quality = resolveStudioQuality(raw ?? 'production'); } catch { quality = resolveStudioQuality('production'); }
  const choice = STUDIO_QUALITY_CHOICES.find(c => c.id === quality.preset) ?? STUDIO_QUALITY_CHOICES[2]!;
  const limit = quality.timeLimitSeconds;
  const perFrame = limit > 0 ? `Up to about ${duration(limit)} per frame` : `No time limit · ${quality.maxSamples} samples per frame`;
  const count = Number.isFinite(frames) && (frames as number) > 1 ? Math.round(frames as number) : 0;
  const total = count && limit > 0 ? `Up to about ${duration(limit * count)} for ${count} frames` : null;
  return { label: choice.label, summary: choice.summary, perFrame, total };
}
