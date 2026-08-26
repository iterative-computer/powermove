import { describe, expect, it } from 'vitest';

import { makePM } from './make-pm';

describe('isolated scripting oracle survivor', () => {
  it('generates a parseable, network-blocked, worker-timed opaque frame', () => {
    const PM = makePM('core/scripting');

    expect(() => new Function(PM.Script.test.workerSource)).not.toThrow();
    const frame = PM.Script.test.frameDocument();

    expect(frame).toMatch(/default-src 'none'/);
    expect(frame).toMatch(/connect-src 'none'/);
    expect(frame).toMatch(/worker-src blob:/);
    expect(frame).toMatch(/worker\.terminate\(\)/);
    expect(PM.Script.catalog().guarantees).toContain('opaque origin');
    expect(PM.Script.catalog().guarantees).toContain('network blocked');
    expect(PM.Script.catalog().guarantees).toContain('timeout worker');
  });
});
