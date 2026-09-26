import { expect, test } from 'bun:test';
import { parseMultipart } from '../src/objects/multipart';
const enc = new TextEncoder();
function chunks(text: string, width: number) {
  const bytes = enc.encode(text);
  return new ReadableStream<Uint8Array>({
    start(c) {
      for (let i = 0; i < bytes.length; i += width) {
        c.enqueue(bytes.slice(i, i + width));
      }
      c.close();
    },
  });
}
const part =
  'Content-Disposition: form-data; name="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; filename="x"\r\nContent-Type: application/x-git-loose-object\r\n\r\nabc';
test('multipart handles split boundary, preamble, epilogue and CRLF', async () => {
  const text = 'preamble\r\n--test\r\n' + part + '\r\n--test--\r\nepilogue';
  const found = [];
  for await (const p of parseMultipart(chunks(text, 3), 'multipart/form-data; boundary="test"')) {
    found.push(p);
  }
  expect(found).toHaveLength(1);
  expect(found[0]?.name).toBe('a'.repeat(40));
  expect(new TextDecoder().decode(found[0]?.bytes)).toBe('abc');
});
test('multipart rejects missing terminal boundary', async () => {
  const text = '--test\r\n' + part;
  const read = async () => {
    for await (const _ of parseMultipart(chunks(text, 5), 'multipart/form-data; boundary=test')) {
    }
  };
  expect(read()).rejects.toMatchObject({ body: { error: 'bad_request' } });
});
