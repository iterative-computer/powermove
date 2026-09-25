import { expect, it } from 'vitest';

import { blocksFromMarkdown } from '../../panels/agent/markdown';
import { normalizeAgentSummary } from './agent-prose';

it('retains a structured result summary as renderable Markdown', () => {
  const summary = normalizeAgentSummary('## Done\r\n\r\n- **One**\r\n- Two');
  expect(blocksFromMarkdown(summary).map(block => block.kind)).toEqual(['h', 'li', 'li']);
  expect(summary).toBe('## Done\n\n- **One**\n- Two');
});
