import type { TranscriptionBridge } from '../../../shared/transcription';
import { bridge } from '../kernel/bridge';

/** The host's transcription bridge, or null where there is none (unit tests, old hosts). */
export function transcriptionBridge(): TranscriptionBridge | null {
  const candidate = bridge()?.transcription;
  return candidate && typeof candidate.status === 'function' ? candidate : null;
}
