import { describe, expect, it } from 'vitest';
import { orchestrationProviders } from './orchestration-providers';

describe('subagent provider discovery', () => {
  it('uses the picker catalog including discovered custom models and their options', async () => {
    const catalog = await orchestrationProviders({
      chatgpt: { status: async () => ({ state: 'connected' } as any), models: async () => [{ id: 'custom-codex', label: 'Custom Codex', reasoningEfforts: ['high'] }] },
      claude: { status: async () => ({ state: 'connected' } as any), models: async () => [{ id: 'custom-claude', label: 'Custom Claude', reasoningEfforts: ['low', 'medium'] }] },
      compatible: { status: async () => ({ baseUrl: 'https://example.test/v1', hasKey: true, vision: false, model: 'local-model', models: ['local-model', 'other-model'] }) }
    });
    expect(catalog.every(provider => provider.canRunChildTask && provider.canRunCrossProviderChildTask)).toBe(true);
    expect(catalog[0]!.models.find(model => model.id === 'custom-codex')?.options[0]?.options).toEqual([{ id: 'high', label: 'High' }]);
    expect(catalog[1]!.models.find(model => model.id === 'custom-claude')?.options[0]?.options.map(option => option.id)).toEqual(['low', 'medium']);
    expect(catalog[2]!.models.map(model => model.id)).toEqual(['local-model', 'other-model']);
    expect(JSON.stringify(catalog)).not.toContain('https://example.test');
    expect(JSON.stringify(catalog)).not.toContain('hasKey');
  });

  it('reports disconnected providers and tolerates unavailable model discovery', async () => {
    const catalog = await orchestrationProviders({
      chatgpt: { status: async () => ({ state: 'disconnected' } as any), models: async () => { throw new Error('Offline'); } },
      claude: { status: async () => { throw new Error('Offline'); } },
      compatible: { status: async () => ({ baseUrl: 'http://localhost:11434/v1', hasKey: false, vision: false, model: '' }) }
    });
    expect(catalog.every(provider => !provider.canRunChildTask && provider.constraints.length)).toBe(true);
    expect(catalog[0]!.models.length).toBeGreaterThan(0);
    expect(catalog[2]!.models).toEqual([]);
  });
});
