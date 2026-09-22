import type { AgentSnapshot } from './agent-state.svelte';

export interface SlashCommand {
  id: string;
  label: string;
  description: string;
  action: 'new' | 'model' | 'provider' | 'effort' | 'attach' | 'stop' | 'settings';
  value?: string;
}

export function slashCommands(draft: string, state: Pick<AgentSnapshot,
  'models' | 'providers' | 'provider' | 'reasoningEfforts' | 'threadSwitchBlocked' | 'legacyPhase'>): SlashCommand[] {
  if (!draft.startsWith('/') || /[\r\n]/.test(draft)) return [];
  const [command = '', ...args] = draft.slice(1).toLowerCase().split(/\s+/);
  if (args.length) {
    const query = args.join(' ').trim();
    const choices = command === 'model' ? state.models
      : command === 'provider' ? state.providers
      : command === 'effort' ? state.reasoningEfforts.map(id => ({ id, label: id === 'xhigh' ? 'Extra high' : id.charAt(0).toUpperCase() + id.slice(1) })) : [];
    const matching = choices.filter(item => `${item.id} ${item.label}`.toLowerCase().includes(query)).map(item => ({
      id: `${command}-${item.id}`, label: item.label, description: `Set ${command}`,
      action: command as 'model' | 'provider' | 'effort', value: item.id,
    }));
    if (command === 'model' && state.provider === 'claude' && /^claude-[a-z0-9-]{1,100}$/.test(query)
      && !state.models.some(item => item.id === query)) {
      matching.push({ id: `model-${query}`, label: query, description: 'Use Claude model ID', action: 'model', value: query });
    }
    return matching;
  }
  const working = state.legacyPhase === 'working';
  const commands: SlashCommand[] = [
    /* A working thread keeps working in the background, so /new stays open —
       only an in-flight apply, which is mutating the editor, holds it back. */
    ...(!state.threadSwitchBlocked && state.legacyPhase !== 'applying'
      ? [{ id: 'new', label: '/new',
          description: working ? 'Start a new conversation and leave this one running' : 'Start a new conversation',
          action: 'new' as const }] : []),
    { id: 'model', label: '/model', description: 'Choose a model', action: 'model' },
    { id: 'effort', label: '/effort', description: 'Set reasoning effort', action: 'effort' },
    { id: 'provider', label: '/provider', description: 'Switch AI provider', action: 'provider' },
    { id: 'attach', label: '/attach', description: 'Attach files or images', action: 'attach' },
    { id: 'settings', label: '/settings', description: 'Open agent connections', action: 'settings' },
    ...(working ? [{ id: 'stop', label: '/stop', description: 'Stop the current run', action: 'stop' as const }] : []),
  ];
  return commands.filter(item => item.id.startsWith(command));
}
