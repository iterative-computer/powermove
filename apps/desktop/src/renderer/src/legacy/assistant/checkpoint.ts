import type { PMRegistry } from '../registry';

/** A run's Undo fallback belongs to that run, not the user's saved Takes archive.
 * Keep the snapshot out of enumerable UI state so progress updates don't copy it.
 * Run results are already transient and are not restored from saved conversations. */
export interface AgentCheckpoint {
  id: string;
  label: string;
  readonly json: string;
  historyId?: string | null;
}

export function createAgentCheckpoint(
  PM: PMRegistry,
  label: string
): AgentCheckpoint {
  const checkpoint = { id: PM.uid('agent-checkpoint'), label };
  return Object.defineProperty(checkpoint, 'json', {
    value: JSON.stringify(PM.proj),
    enumerable: false
  }) as AgentCheckpoint;
}
