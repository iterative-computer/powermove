import { describe, expect, it } from 'vitest';

import type { PublishPlanDto } from '../../../shared/publish';
import { canPublish, checkIconBytes, draftFor, formFor, problemsOf, progressText, versionProblem, waiverProblem } from './publish-form';

function plan(overrides: Partial<PublishPlanDto> = {}): PublishPlanDto {
  return {
    localId: 'glass-blur', coordinate: 'jude/glass-blur', version: '1.0.0', suggestedVersion: '1.0.0', lastVersion: null,
    firstPublish: true, treeSha: 'a'.repeat(40), fileCount: 3, sizeBytes: 120, isFork: false,
    blockedFindings: [], permissionFindings: [], waivableFindings: [],
    manifest: { id: 'glass-blur', name: 'Glass blur', description: 'Frosted glass.' },
    listing: { name: 'Glass blur', tagline: 'Frosted glass.', category: 'effects', licence: 'MIT' },
    ...overrides
  };
}

function png(width: number, height: number, size = 64): Uint8Array {
  const bytes = new Uint8Array(size);
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82]);
  const view = new DataView(bytes.buffer);
  view.setUint32(16, width);
  view.setUint32(20, height);
  return bytes;
}

describe('the version field', () => {
  it('takes three numbers', () => {
    expect(versionProblem('1.2.0', null)).toBeNull();
    expect(versionProblem('', null)).toBe('Enter a version.');
    expect(versionProblem('1.2', null)).toBe('Use three numbers, like 1.2.0.');
    expect(versionProblem('v1.2.0', null)).toBe('Use three numbers, like 1.2.0.');
    expect(versionProblem('1.2.0-beta', null)).toBe('Use three numbers, like 1.2.0.');
  });

  it('must be higher than the last published version, compared as numbers', () => {
    expect(versionProblem('1.2.0', '1.2.0')).toBe('Use a version higher than 1.2.0.');
    expect(versionProblem('1.1.9', '1.2.0')).toBe('Use a version higher than 1.2.0.');
    expect(versionProblem('1.10.0', '1.9.0')).toBeNull();
    expect(versionProblem('2.0.0', '1.99.99')).toBeNull();
  });

  it('prefills what main suggests', () => {
    expect(draftFor(plan({ lastVersion: '1.0.0', suggestedVersion: '1.0.1', firstPublish: false })).version).toBe('1.0.1');
    expect(problemsOf(draftFor(plan({ lastVersion: '1.0.0', suggestedVersion: '1.0.1', firstPublish: false })), plan({ lastVersion: '1.0.0' })).version).toBeUndefined();
  });
});

describe('scanner findings', () => {
  const soft = { path: 'index.ts', line: 4, kind: 'high_entropy' as const };

  it('needs a reason of at least three characters for each waivable finding', () => {
    expect(waiverProblem(undefined)).toBe('Say why this is safe to publish.');
    expect(waiverProblem('  ok ')).toBe('Say why this is safe to publish.');
    expect(waiverProblem('test fixture')).toBeNull();
    expect(waiverProblem('x'.repeat(201))).toBe('Keep the reason under 200 characters.');

    const withFinding = plan({ waivableFindings: [soft] });
    const draft = draftFor(withFinding);
    expect(problemsOf(draft, withFinding).reasons).toEqual({ 'index.ts:4': 'Say why this is safe to publish.' });
    expect(canPublish(problemsOf(draft, withFinding))).toBe(false);
    draft.reasons['index.ts:4'] = 'a hash, not a key';
    expect(canPublish(problemsOf(draft, withFinding))).toBe(true);
    expect(formFor(draft, withFinding).waivers).toEqual([{ path: 'index.ts', line: 4, reason: 'a hash, not a key' }]);
  });

  it('prefills a reason already written in the file', () => {
    const withReason = plan({ waivableFindings: [{ ...soft, reason: 'fixture hash' }] });
    expect(canPublish(problemsOf(draftFor(withReason), withReason))).toBe(true);
  });

  it('blocks undeclared permissions in the publish plan', () => {
    const missing = plan({ permissionFindings: [{ path: 'index.ts', line: 12, needs: 'network', text: 'Declare network.' }] });
    expect(canPublish(problemsOf(draftFor(missing), missing))).toBe(false);
  });

  it('never lets a hard finding through', () => {
    const blocked = plan({ blockedFindings: [{ path: 'index.ts', line: 1, kind: 'aws_access_key' }] });
    const problems = problemsOf(draftFor(blocked), blocked);
    expect(problems.blocked).toBe(true);
    expect(canPublish(problems)).toBe(false);
  });
});

describe('the form sent to main', () => {
  it('sends the listing, visibility and icon on a first publish only', () => {
    const first = plan();
    const draft = { ...draftFor(first), notes: '  First version. ', visibility: 'unlisted' as const, iconPng: 'iVBORw0KGgo=' };
    expect(formFor(draft, first)).toEqual({
      version: '1.0.0', notes: 'First version.', waivers: [], visibility: 'unlisted', iconPng: 'iVBORw0KGgo=',
      listing: { name: 'Glass blur', tagline: 'Frosted glass.', category: 'effects', licence: 'MIT' }
    });
    const update = plan({ firstPublish: false, lastVersion: '1.0.0', suggestedVersion: '1.0.1' });
    expect(formFor({ ...draftFor(update), iconPng: 'iVBORw0KGgo=' }, update)).toEqual({ version: '1.0.1', waivers: [] });
  });

  it('needs a name on a first publish', () => {
    const first = plan();
    expect(problemsOf({ ...draftFor(first), name: '  ' }, first).name).toBe('Enter a name.');
    expect(problemsOf({ ...draftFor(first), name: '' }, plan({ firstPublish: false })).name).toBeUndefined();
  });

  it('checks the icon the way main will', () => {
    expect(checkIconBytes(png(256, 256)).ok).toBe(true);
    expect(checkIconBytes(png(256, 128))).toEqual({ ok: false, error: 'Choose a square image.' });
    expect(checkIconBytes(new Uint8Array(64))).toEqual({ ok: false, error: 'Choose a PNG file.' });
    expect(checkIconBytes(png(256, 256, 256 * 1024 + 1))).toEqual({ ok: false, error: 'Choose a PNG of 256 KB or smaller.' });
  });

  it('reports upload progress in files', () => {
    expect(progressText({ localId: 'glass-blur', phase: 'uploading', done: 12, total: 40 })).toBe('Uploading 12 of 40 files…');
    expect(progressText({ localId: 'glass-blur', phase: 'publishing', done: 0, total: 0 })).toBe('Publishing…');
  });
});
