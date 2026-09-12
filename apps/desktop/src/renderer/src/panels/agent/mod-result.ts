export interface AgentModResult {
  id: string;
  name: string;
  action: 'created' | 'updated' | 'removed';
  status: 'ready' | 'error' | 'removed';
}

export function modResultForMessage(
  message: { role: string; text?: string; modResult?: AgentModResult },
  panels: Array<{ ownerId?: string }>
): AgentModResult | null {
  const result = message.modResult;
  // Only successful typed agent results backed by registered panels get a card.
  if (message.role !== 'assistant' || !result
    || typeof result.id !== 'string' || typeof result.name !== 'string'
    || !['created', 'updated'].includes(result.action) || result.status !== 'ready') return null;
  return panels.some(panel => panel.ownerId === result.id) ? result : null;
}
