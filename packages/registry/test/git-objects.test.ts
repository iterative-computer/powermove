import { describe, expect, test } from 'bun:test';
import { bytesToHex, decodeLoose, encodeCommit, encodeLoose, encodeTree, GitCodecError, hashObject, hexToBytes, isSha1Hex, isSha256Hex, parseCommit, parseTree, sha256Hex } from '../src/git/objects';

const enc = new TextEncoder();
const empty = new Uint8Array();
const sha = 'e69de29bb2d1d6434b8b29ae775ad8c2e48c5391';

describe('git objects', () => {
  test('known hashes and hex helpers', async () => {
    expect(await hashObject('blob', empty)).toBe(sha);
    expect(await hashObject('tree', empty)).toBe('4b825dc642cb6eb9a060e54bf8d69288fbee4904');
    expect(await sha256Hex(empty)).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(bytesToHex(hexToBytes(sha))).toBe(sha);
    expect(isSha1Hex(sha)).toBe(true);
    expect(isSha256Hex(await sha256Hex(empty))).toBe(true);
    expect(() => hexToBytes('xz')).toThrow(GitCodecError);
  });

  test('zlib loose round trip', async () => {
    const body = enc.encode('hello\n');
    expect(await decodeLoose(await encodeLoose('blob', body), 100)).toEqual({ type: 'blob', body });
  });

  test('rejects malformed headers and mismatched lengths', async () => {
    async function packed(raw: string) {
      const stream = new CompressionStream('deflate');
      const reading = new Response(stream.readable).arrayBuffer();
      const writer = stream.writable.getWriter();
      await writer.write(enc.encode(raw)); await writer.close();
      return new Uint8Array(await reading);
    }
    for (const raw of ['tag 0\0', 'blob xx\0', 'blob 00\0', 'blob 0', 'blob 1\0']) {
      await expect(decodeLoose(await packed(raw), 100)).rejects.toMatchObject({ code: raw === 'blob 1\0' ? 'length_mismatch' : 'bad_header' });
    }
    await expect(decodeLoose(new Uint8Array([1, 2, 3]), 100)).rejects.toMatchObject({ code: 'inflate_failed' });
  });

  test('inflation stops at cap', async () => {
    const packed = await encodeLoose('blob', new Uint8Array(100_000));
    await expect(decodeLoose(packed, 40)).rejects.toMatchObject({ code: 'too_large' });
  });

  test('trees use Git directory sorting and reject noncanonical entries', () => {
    const entries = [
      { mode: '40000' as const, name: 'a', sha },
      { mode: '100644' as const, name: 'b', sha },
      { mode: '100644' as const, name: 'a.txt', sha },
      { mode: '100644' as const, name: 'a-b', sha },
    ];
    const body = encodeTree(entries);
    expect(parseTree(body).map((entry) => `${entry.name}${entry.mode === '40000' ? '/' : ''}`)).toEqual(['a-b', 'a.txt', 'a/', 'b']);
    expect(() => encodeTree([entries[0]!, entries[0]!])).toThrow(GitCodecError);
    expect(() => encodeTree([{ mode: '100644', name: 'a/b', sha }])).toThrow(GitCodecError);
    const first = encodeTree([entries[1]!]);
    const second = encodeTree([entries[2]!]);
    const backwards = new Uint8Array(first.length + second.length);
    backwards.set(first); backwards.set(second, first.length);
    expect(() => parseTree(backwards)).toThrow(GitCodecError);
  });

  test('commit round trip and identity validation', () => {
    const identity = { name: 'Ada Lovelace', email: 'ada@example.com', time: 1_700_000_000, tz: '+0000' };
    const commit = { tree: sha, parents: [sha], author: identity, committer: identity, message: 'Release\n\nDetails\n' };
    expect(parseCommit(encodeCommit(commit))).toEqual(commit);
    expect(() => encodeCommit({ ...commit, author: { ...identity, name: 'Bad\nName' } })).toThrow(GitCodecError);
    expect(() => parseCommit(enc.encode(`tree ${sha}\nunknown value\n\nmessage`))).toThrow(GitCodecError);
  });
});
