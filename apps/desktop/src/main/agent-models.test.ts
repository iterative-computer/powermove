import { expect, it } from 'vitest';
import { AGENT_MODELS, modelEffort, modelEfforts, setDiscoveredCodexModels } from '../shared/agent-models';
import { buildClaudeArgv } from './claude/adapter';
import { isCodexRunRequest } from './codex/runner';

it('passes every listed Claude model to the native runtime with supported effort', () => {
  for (const model of AGENT_MODELS.claude) {
    const args = buildClaudeArgv({ model: model.id, reasoningEffort: 'xhigh', schema: {}, prompt: 'test', imagePaths: [], sessionId: null, access: 'editor' });
    expect(args[args.indexOf('--model') + 1]).toBe(model.id);
    const effort = modelEffort('claude', model.id, 'xhigh');
    if (effort) expect(args[args.indexOf('--effort') + 1]).toBe(effort);
    else expect(args).not.toContain('--effort');
  }
  expect(modelEfforts('claude', 'haiku')).toEqual([]);
  expect(modelEffort('claude', 'claude-sonnet-4-6', 'xhigh')).toBe('high');
});

it('accepts Astra requests at every exposed effort through IPC validation', () => {
  for (const reasoningEffort of modelEfforts('chatgpt', 'gpt-6-astra')) {
    expect(isCodexRunRequest({
      id: 'astra-model-test', provider: 'chatgpt', mode: 'autonomous', prompt: 'test',
      schema: null, images: [], model: 'gpt-6-astra', reasoningEffort, access: 'project',
      projectId: 'test', projectName: 'Test', projectJSON: '{}', attachments: [], consentToken: null,
    })).toBe(true);
  }
});

it('uses discovered Codex models and their available efforts', () => {
  setDiscoveredCodexModels([{
    id: 'gpt-6-astra', label: 'GPT-6 Astra', reasoningEfforts: ['low', 'medium', 'max']
  }, {
    id: 'gpt-5.5', label: 'GPT-5.5', reasoningEfforts: ['none', 'low', 'medium']
  }]);
  expect(AGENT_MODELS.chatgpt.map(model => model.id)).toEqual([
    'gpt-6-astra', 'gpt-6-sol', 'gpt-6-luna', 'gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-5.5'
  ]);
  expect(modelEfforts('chatgpt', 'gpt-6-astra')).toEqual(['low', 'medium', 'max']);
  expect(modelEffort('chatgpt', 'gpt-6-astra', 'high')).toBe('medium');
  expect(modelEfforts('chatgpt', 'gpt-6-sol')).toEqual(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
  expect(modelEfforts('chatgpt', 'gpt-5.5')).toEqual(['none', 'low', 'medium']);
  expect(modelEfforts('compatible', 'gpt-6-sol')).toEqual(['none', 'low', 'medium', 'high', 'xhigh', 'max']);
  expect(modelEfforts('claude', 'claude-opus-5-5')).not.toContain('none');
  expect(modelEfforts('claude', 'claude-opus-5-5')).not.toContain('ultra');
  expect(isCodexRunRequest({
    id: 'sol-model-test', provider: 'chatgpt', mode: 'autonomous', prompt: 'test',
    schema: null, images: [], model: 'gpt-6-sol', reasoningEffort: 'ultra', access: 'project',
    projectId: 'test', projectName: 'Test', projectJSON: '{}', attachments: [], consentToken: null,
  })).toBe(true);
});
