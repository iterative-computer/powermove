import { AGENT_MODELS, modelEfforts, setDiscoveredClaudeModels, setDiscoveredCodexModels } from '../shared/agent-models';
import type { OrchestrationProvider } from '../shared/agent-orchestration';
import type { CompatibleProviderConfig } from '../shared/compatible-provider';
import type { ChatGPTAccountStatus, ClaudeAccountStatus, ClaudeModelOption, CodexModelOption } from '../shared/ipc';

export async function orchestrationProviders(options: {
  chatgpt: { status(): Promise<ChatGPTAccountStatus>; models?(): Promise<CodexModelOption[]> };
  claude: { status(): Promise<ClaudeAccountStatus>; models?(): Promise<ClaudeModelOption[]> };
  compatible: { status(): Promise<CompatibleProviderConfig> };
}): Promise<OrchestrationProvider[]> {
  const [chatgpt, claude, compatible, codexModels, claudeModels] = await Promise.allSettled([
    options.chatgpt.status(), options.claude.status(), options.compatible.status(),
    options.chatgpt.models?.() ?? Promise.resolve([]), options.claude.models?.() ?? Promise.resolve([])
  ]);
  if (codexModels.status === 'fulfilled') setDiscoveredCodexModels(codexModels.value);
  if (claudeModels.status === 'fulfilled') setDiscoveredClaudeModels(claudeModels.value);
  const config = compatible.status === 'fulfilled' ? compatible.value : null;
  return (['chatgpt', 'claude', 'compatible'] as const).map(provider => {
    const connected = provider === 'chatgpt' ? chatgpt.status === 'fulfilled' && chatgpt.value.state === 'connected'
      : provider === 'claude' ? claude.status === 'fulfilled' && claude.value.state === 'connected' : Boolean(config?.model);
    const models = provider === 'compatible'
      ? config?.model ? [...new Set([config.model, ...(config.models ?? [])])].map(id => ({ id, label: id })) : []
      : AGENT_MODELS[provider];
    return {
      providerInstanceId: provider, driverKind: provider === 'chatgpt' ? 'codex' : provider,
      displayName: provider === 'chatgpt' ? 'ChatGPT' : provider === 'claude' ? 'Claude' : 'Connected model',
      canRunChildTask: connected, canRunCrossProviderChildTask: connected,
      constraints: connected ? [] : ['Connect this provider in Settings before delegating work.'],
      models: models.map(model => {
        const efforts = modelEfforts(provider, model.id);
        return { ...model, options: efforts.length ? [{
          id: 'reasoningEffort' as const, label: 'Reasoning', type: 'select' as const,
          options: efforts.map(id => ({ id, label: id === 'xhigh' ? 'Extra High' : id[0]!.toUpperCase() + id.slice(1) }))
        }] : [] };
      })
    };
  });
}
