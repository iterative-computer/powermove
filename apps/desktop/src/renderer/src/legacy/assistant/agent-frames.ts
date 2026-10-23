import type { EngineBridge } from '../../../../shared/engine-ipc';

/** Captures the document once before any batch, so edits during an observation
 * cannot turn a contact sheet into a mixture of project revisions. */
export async function captureAgentFrames(PM: any, native: EngineBridge | undefined, times: number[], width: number): Promise<{ times: number[]; images: string[] }> {
  await PM.EngineDoc?.whenReady;
  await PM.HostMedia?.flushBindings?.();
  await PM.EngineDefinitions?.settle?.();
  if (!native?.agentFrames) throw new Error('agent-frames-unavailable: The native agent frame service is unavailable.');
  const project = JSON.stringify(PM.proj);
  const revision = Number(PM.proj?.revision) || 0;
  const images: string[] = [];
  for (let offset = 0; offset < times.length; offset += 5) {
    const batch = times.slice(offset, offset + 5);
    const result = await native.agentFrames({ project, revision, times: batch, width });
    if (result.status !== 'rendered' || result.images.length !== batch.length) throw new Error('agent-frames-incomplete: The engine did not render every requested frame.');
    for (let index = 0; index < result.images.length; index++) {
      const frame = result.images[index]!;
      if (frame.time !== batch[index]) throw new Error('agent-frames-incomplete: The engine returned a different frame time.');
      let binary = '';
      // Chunks keep argument counts bounded even at the frame response cap.
      for (let start = 0; start < frame.data.length; start += 8192) binary += String.fromCharCode(...frame.data.subarray(start, start + 8192));
      images.push(`data:${frame.mimeType};base64,${btoa(binary)}`);
    }
  }
  return { times, images };
}
