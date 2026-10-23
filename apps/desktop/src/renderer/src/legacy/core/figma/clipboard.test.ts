import { describe, expect, it } from 'vitest';
import { deflateRawSync } from 'node:zlib';
import { encodeBinarySchema, parseSchema } from '@open-pencil/kiwi/schema-runtime';
import { archiveHtml, figmaHtml, figmaNode, pathBlob } from './__fixtures__/clipboard';
import { decodeFigmaClipboard } from './clipboard';

describe('Figma clipboard decoder', () => {
  it.each(['deflate', 'zstd'] as const)('reads %s selections using their embedded schema, including unicode and binary geometry', async compression => {
    const node = figmaNode(1, 'VECTOR', { name: 'Flèche → 日本語', fillGeometry: [{ commandsBlob: 0 }] });
    const selection = await decodeFigmaClipboard(await figmaHtml([node], [pathBlob()], compression));
    expect(selection.nodes[0]).toMatchObject(node);
    expect(selection.blobs[0]).toEqual(pathBlob());
  });
  it.each(['', '<p>normal HTML</p>', '<!--(figma)bad!(/figma)-->', '<!--(figma)AA==(/figma)-->'])('rejects missing or malformed data without reading HTML as code', async html => {
    await expect(decodeFigmaClipboard(html)).rejects.toThrow();
  });
  it('rejects truncated chunks and large node selections', async () => {
    const html = await figmaHtml();
    const encoded = /\(figma\)(.*?)\(\/figma\)/.exec(html)![1]!;
    const bytes = Buffer.from(encoded, 'base64');
    bytes.writeUInt32LE(0xffffffff, 12);
    await expect(decodeFigmaClipboard(`(figma)${bytes.toString('base64')}(/figma)`)).rejects.toThrow('incomplete');
    await expect(decodeFigmaClipboard(await figmaHtml(Array.from({ length: 5001 }, (_, i) => figmaNode(i))))).rejects.toThrow('5,000');
  });
  it('bounds malicious array lengths before allocation', async () => {
    const schema = encodeBinarySchema(parseSchema('message Message { uint[] values = 1; }'));
    const payload = Uint8Array.from([1, 255, 255, 255, 255, 15]);
    await expect(decodeFigmaClipboard(archiveHtml(deflateRawSync(schema), deflateRawSync(payload)))).rejects.toThrow('array length');
  });
  it('rejects incomplete schemas without hanging', async () => {
    await expect(decodeFigmaClipboard(archiveHtml(deflateRawSync(Uint8Array.from([1, 65])), deflateRawSync(Uint8Array.from([0]))))).rejects.toThrow('schema');
  });
  it('rejects an oversized zstd history window before allocating it', async () => {
    const schema = encodeBinarySchema(parseSchema('message Message { uint value = 1; }'));
    const frame = Uint8Array.from([0x28, 0xb5, 0x2f, 0xfd, 0, 0xff]);
    await expect(decodeFigmaClipboard(archiveHtml(deflateRawSync(schema), frame))).rejects.toThrow('too large');
  });
});
