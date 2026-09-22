import type { CodexModelOption, ReasoningEffort } from './ipc';

// These entries keep the picker usable when live Codex discovery is unavailable.
// Claude aliases resolve to the current model supported by the installed CLI.
export const AGENT_MODELS = {
  compatible: [{ id: 'configured', label: 'Connected model' }],
  chatgpt: [
    { id: 'gpt-6-astra', label: 'GPT 6 Astra' },
    { id: 'gpt-6-sol', label: 'GPT 6 Sol' },
    { id: 'gpt-5.6-sol', label: '5.6 Sol' },
    { id: 'gpt-5.6-terra', label: '5.6 Terra' },
    { id: 'gpt-5.6-luna', label: '5.6 Luna' },
  ],
  claude: [
    { id: 'claude-fable-5-1', label: 'Fable 5.1' },
    { id: 'claude-opus-5-5', label: 'Opus 5.5' },
    { id: 'claude-opus-5', label: 'Opus 5' },
    { id: 'claude-sonnet-5', label: 'Sonnet 5' },
    { id: 'claude-haiku-4-5-20251001', label: 'Haiku 4.5' },
    { id: 'claude-fable-5', label: 'Fable 5' },
    { id: 'claude-opus-4-8', label: 'Opus 4.8' },
    { id: 'claude-opus-4-7', label: 'Opus 4.7' },
    { id: 'claude-opus-4-6', label: 'Opus 4.6' },
    { id: 'claude-sonnet-4-6', label: 'Sonnet 4.6' },
    { id: 'sonnet', label: 'Sonnet (latest)' },
    { id: 'opus', label: 'Opus (latest)' },
    { id: 'fable', label: 'Fable (latest)' },
    { id: 'haiku', label: 'Haiku (latest)' },
  ],
};

export const REASONING_EFFORTS: ReasoningEffort[] = ['low', 'medium', 'high', 'xhigh', 'max'];
const discoveredCodexEfforts = new Map<string, ReasoningEffort[]>();

export function setDiscoveredCodexModels(models: CodexModelOption[]): void {
  if (!models.length) return;
  discoveredCodexEfforts.clear();
  AGENT_MODELS.chatgpt = models.map(({ id, label, reasoningEfforts }) => {
    discoveredCodexEfforts.set(id, reasoningEfforts);
    return { id, label };
  });
}

export function modelEfforts(provider: string, model: string | null): ReasoningEffort[] {
  if (provider === 'chatgpt' && model && discoveredCodexEfforts.has(model)) {
    return discoveredCodexEfforts.get(model)!;
  }
  if (provider === 'compatible') {
    if (/^gpt-(6-(?:astra|sol)|5\.6)(?:-|$)/.test(model || '')) return REASONING_EFFORTS;
    if (/^gpt-5\.[2345]-chat(?:-|$)/.test(model || '')) return [];
    if (/^gpt-5\.[245]-pro(?:-|$)/.test(model || '')) return ['medium', 'high', 'xhigh'];
    if (/^gpt-5\.[2345](?:-|$)/.test(model || '')) return ['low', 'medium', 'high', 'xhigh'];
    if (/^(o[134]|gpt-oss)(?:-|$)/.test(model || '')) return ['low', 'medium', 'high'];
    // Compatible services also host models without this OpenAI parameter.
    return [];
  }
  if (provider !== 'claude') return REASONING_EFFORTS;
  if (model?.includes('haiku')) return [];
  if (model === 'claude-opus-4-6' || model === 'claude-sonnet-4-6') {
    return ['low', 'medium', 'high', 'max'];
  }
  return REASONING_EFFORTS;
}

export function modelEffort(provider: string, model: string | null, effort: ReasoningEffort | null): ReasoningEffort | null {
  if (!effort) return null;
  const supported = modelEfforts(provider, model);
  if (!supported.length) return null;
  if (supported.includes(effort)) return effort;
  const rank = REASONING_EFFORTS.indexOf(effort);
  return REASONING_EFFORTS.filter(item => supported.includes(item)).filter(item => REASONING_EFFORTS.indexOf(item) <= rank).at(-1)
    ?? supported[0]!;
}
