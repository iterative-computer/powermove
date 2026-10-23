import { describe, expect, it } from 'vitest';

import { decodeEnvelope, encodeEnvelope, envelope, eventMessage } from '../../../main/engine/protocol';
import { releaseEnvelope } from './release-envelope';

describe('releaseEnvelope', () => {
  it('writes the bytes the protocol encoder writes for a frame release, at every integer width', () => {
    for (const [session, poolIndex, seq] of [[1, 0, 1], [77, 7, 200], [300, 31, 70_000], [70_000, 3, 5_000_000_000]]) {
      const ours = releaseEnvelope(session!, { poolIndex: poolIndex!, seq: seq! });
      const theirs = encodeEnvelope(envelope(eventMessage({ type: 'releaseFrame', poolIndex, seq } as never), session));
      expect([...ours]).toEqual([...theirs]);
      expect(decodeEnvelope(ours)).toMatchObject({ session, message: { kind: 'event', body: { type: 'releaseFrame', poolIndex, seq } } });
    }
  });
});
