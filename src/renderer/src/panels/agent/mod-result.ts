export interface AgentModResult {
  id: string;
  name: string;
  action: 'created' | 'updated' | 'removed';
  status: 'ready' | 'error' | 'removed';
}

export function modResultForMessage(
  message: { role: string; text?: string; modResult?: AgentModResult },
  records: Array<Record<string, any>>
): AgentModResult | null {
  if (message.role !== 'assistant') return null;
  const result = message.modResult;
  if (result && typeof result.id === 'string' && typeof result.name === 'string'
    && ['created', 'updated', 'removed'].includes(result.action)
    && ['ready', 'error', 'removed'].includes(result.status)) return result;
  // Older app-generated change messages have no typed payload. Resolve only
  // their exact historical form against an installed mod, never arbitrary prose.
  const match = /^(Added|Updated) mod ([^\r\n]+)$/.exec(message.text || '');
  if (!match) return null;
  const record = records.find(item => item.manifest?.name === match[2] || item.id === match[2]);
  return record ? {
    id: record.id, name: record.manifest?.name || record.id,
    action: match[1] === 'Added' ? 'created' : 'updated',
    status: record.health?.error ? 'error' : 'ready'
  } : null;
}
