import { sha256HexOf } from '../../../../shared/sha256';

/** Stable across reopen and independent of playhead, selection, and panel layout. */
export async function projectFingerprint(projectJSON: string): Promise<string> {
  return sha256HexOf(new TextEncoder().encode(projectJSON));
}
