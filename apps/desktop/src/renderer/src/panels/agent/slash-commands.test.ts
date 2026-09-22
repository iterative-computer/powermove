import { expect, it } from 'vitest';
import { slashCommands } from './slash-commands';

it('offers a new Claude model ID as a selectable slash command', () => {
  const state = {
    provider: 'claude',
    models: [{ id: 'opus', label: 'Opus (latest)' }],
    providers: [],
    reasoningEfforts: ['high'],
    threadSwitchBlocked: false,
    legacyPhase: 'conversation' as const
  };
  expect(slashCommands('/model claude-opus-5-6', state)).toContainEqual({
    id: 'model-claude-opus-5-6', label: 'claude-opus-5-6',
    description: 'Use Claude model ID', action: 'model', value: 'claude-opus-5-6'
  });
  expect(slashCommands('/model claude-opus-5-6', { ...state, provider: 'chatgpt' })).toEqual([]);
});
