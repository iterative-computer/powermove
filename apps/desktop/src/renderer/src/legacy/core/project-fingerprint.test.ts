import { createHash } from 'node:crypto';

import { describe, expect, it } from 'vitest';

import { serveFingerprints, type FingerprintReply, type FingerprintRequest } from './fingerprint-digests';
import { fingerprintJSON, projectFingerprint } from './project-fingerprint';

const sha = (text: string) => createHash('sha256').update(text, 'utf8').digest('hex');

describe('project fingerprints', () => {
  it('are SHA-256 of the JSON text, whether given the text or the value', async () => {
    const project = { id: 'p', name: 'Fingerprint é 😀', layers: Array.from({ length: 300 }, (_, index) => ({ id: `L${index}`, p: { x: Array.from({ length: 50 }, (_, k) => ({ t: k, v: k * index })) } })), gone: undefined };
    const text = JSON.stringify(project);
    expect(await projectFingerprint(text)).toBe(sha(text));
    expect(await fingerprintJSON(project)).toBe(sha(text));
  });

  it('never splits a surrogate pair between encoded pieces', async () => {
    const text = 'a'.repeat((1 << 20) - 1) + '😀' + 'b'.repeat(10);
    expect(await projectFingerprint(text)).toBe(sha(text));
  });

  it('digests jobs independently on the worker side', () => {
    const replies: FingerprintReply[] = [];
    const scope = { onmessage: null as ((event: { data: FingerprintRequest }) => void) | null, postMessage: (reply: FingerprintReply) => replies.push(reply) };
    serveFingerprints(scope);
    const send = (data: FingerprintRequest) => scope.onmessage!({ data });
    const bytes = (text: string) => new TextEncoder().encode(text).buffer as ArrayBuffer;
    send({ id: 1, bytes: bytes('{"a":') });
    send({ id: 2, bytes: bytes('[1,') });
    send({ id: 1, bytes: bytes('1}') });
    send({ id: 3, bytes: bytes('dropped') });
    send({ id: 3, cancel: true });
    send({ id: 2, bytes: bytes('2]'), end: true });
    send({ id: 1, end: true });
    expect(replies).toEqual([{ id: 2, hex: sha('[1,2]') }, { id: 1, hex: sha('{"a":1}') }]);
  });
});
