import { describe, expect, it } from 'vitest';

import { uploadWording } from './web-bridge';

describe('upload toasts', () => {
  it('only says a file is opening when the person opened one', () => {
    const open = uploadWording('cut.mov', '12', 'open');
    expect(open.done).toBe('Uploaded cut.mov. Opening on the host…');
    const agent = uploadWording('cut.mov', '12', 'agent');
    expect([agent.start, agent.progress(40), agent.done, agent.failed('offline')]).toEqual([
      'Copying cut.mov (12 MB) to the host for the agent…',
      'Copying cut.mov for the agent… 40%',
      'Copied cut.mov to the host for the agent.',
      'Could not copy cut.mov to the host for the agent: offline'
    ]);
    expect(Object.values(agent).some((value) => String(typeof value === 'function' ? value(1) : value).includes('Opening'))).toBe(false);
  });
});
