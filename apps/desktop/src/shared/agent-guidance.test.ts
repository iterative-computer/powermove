import { describe, expect, it } from 'vitest';
import { agentTaskProfile, resultVerificationPrompt } from './agent-guidance';

describe('task guidance', () => {
  it('keeps scene verification focused on the original brief', () => {
    const prompt = resultVerificationPrompt('Animate the title', [], [], []);
    expect(prompt.startsWith('Animate the title')).toBe(true);
    expect(prompt).toContain('never replay completed edits');
    expect(prompt).toContain('do not claim a visual pass');
    expect(prompt).not.toContain('EXTENSION LOAD REPORT');
    expect(prompt).not.toContain('MEDIA IMPORT REPORT');
    expect(agentTaskProfile(prompt)).toEqual({ creative: true, extensions: false, footage: false });
  });

  it('supplies live checks and deferred edits when an extension needs verification', () => {
    const prompt = resultVerificationPrompt('Create two effects', [], [{ id: 'glow', error: 'Registration failed' }], [{ type: 'add_effect' }]);
    expect(agentTaskProfile(prompt).extensions).toBe(true);
    expect(prompt).toContain('Registration failed');
    expect(prompt).toContain('add_effect');
    expect(prompt).toContain('confirm registration');
    expect(prompt).not.toContain('MEDIA IMPORT REPORT');
  });

  it('confirms imported layers and avoids duplicate imports', () => {
    const prompt = resultVerificationPrompt('Trim the video', [{ layerIds: ['clip'], imported: true }], [], []);
    expect(prompt).toContain('Successful imports must not be submitted again');
    expect(prompt).toContain('clip');
    expect(prompt).not.toContain('EXTENSION LOAD REPORT');
  });
});
