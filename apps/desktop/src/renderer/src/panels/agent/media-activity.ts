import { mediaToolLabels, mediaToolName, type AgentMediaToolSubject, type MediaToolLabels } from '../../../../shared/media-tools';

/* Media tool rows name the clip they work on ("Transcribing interview.mov…").
   Main only knows ids; the project knows names. */

type Registry = Record<string, any>;

export function mediaSubjectName(PM: Registry | undefined, subject: AgentMediaToolSubject | undefined): string | null {
  if (!PM || !subject) return null;
  if (subject.layerId) {
    const layer = typeof PM.L === 'function' ? PM.L(subject.layerId) : null;
    if (layer?.name) return String(layer.name);
  }
  const assetId = subject.assetId ?? (subject.layerId && typeof PM.L === 'function' ? PM.L(subject.layerId)?.d?.asset : undefined);
  const asset = assetId ? PM.proj?.assets?.[assetId] : null;
  return asset?.name ? String(asset.name) : null;
}

/** Live labels for a media tool's tool-start event, or null for any other tool. */
export function mediaStepLabels(PM: Registry | undefined, step: { toolName: string; subject?: AgentMediaToolSubject }): MediaToolLabels | null {
  if (!mediaToolName(step.toolName)) return null;
  return mediaToolLabels(step.toolName, step.subject, mediaSubjectName(PM, step.subject));
}
