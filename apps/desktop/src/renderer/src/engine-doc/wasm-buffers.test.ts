import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { imports, PmWasm } from './wasm';

function inputHarness() {
  const memory = new WebAssembly.Memory({ initial: 1 });
  const ptr = 1024;
  const reserve = vi.fn((len: number) => {
    const pages = Math.ceil((ptr + len) / 65536);
    if (pages > memory.buffer.byteLength / 65536) memory.grow(pages - memory.buffer.byteLength / 65536);
    return ptr;
  });
  const wasm = new PmWasm({ exports: { memory, pm_in: reserve } } as unknown as WebAssembly.Instance);
  return { wasm, memory, ptr, reserve };
}

describe('WASM transfer buffers', () => {
  it('counts UTF-8 exactly, including replacement of unpaired UTF-16 surrogates', () => {
    const cases = ['', 'ASCII', '\u007f\u0080\u07ff\u0800\uffff', '🦀漢字é', '\ud800', '\udfff', '\ud800x\udfff', '\ud800\ud800\udc00\udc00', Array.from({ length: 65536 }, (_, code) => String.fromCharCode(code)).join('')];
    // Also sample surrogate adjacency across the UTF-16 range against
    // TextEncoder's own replacement semantics.
    for (let code = 0; code <= 0xffff; code += 251) cases.push(String.fromCharCode(code, 0xd800, code, 0xdc00));
    for (const text of cases) {
      const expected = new TextEncoder().encode(text);
      const { wasm, memory, ptr, reserve } = inputHarness();
      expect(wasm.writeString(text)).toBe(expected.length);
      expect(reserve).toHaveBeenCalledWith(Math.max(1024, expected.length));
      expect(new Uint8Array(memory.buffer, ptr, expected.length)).toEqual(expected);
    }
  });

  it('reserves one byte per ASCII character without allocating a temporary encoded copy', () => {
    const text = 'a'.repeat(8 * 1024 * 1024);
    const { wasm, memory, ptr, reserve } = inputHarness();
    const encode = vi.spyOn(TextEncoder.prototype, 'encode');
    try {
      expect(wasm.writeString(text)).toBe(text.length);
      expect(reserve).toHaveBeenCalledExactlyOnceWith(text.length);
      expect(encode).not.toHaveBeenCalled();
      expect(memory.buffer.byteLength).toBeLessThan(text.length + 2 * 65536);
      expect(new TextDecoder().decode(new Uint8Array(memory.buffer, ptr, text.length))).toBe(text);
    } finally { encode.mockRestore(); }
  });

  it('roundtrips large ASCII through real wasm with bounded input and one canonical output allocation', async () => {
    const module = await WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../public/engine/pm_wasm.wasm', import.meta.url))));
    const wasm = new PmWasm(await WebAssembly.instantiate(module, imports));
    const text = JSON.stringify({ data: Array.from({ length: 8192 }, () => 'a'.repeat(1024)) });
    wasm.resync(text, 1);
    const input = wasm.heap();
    expect(input.input).toBe(text.length);
    wasm.releaseInput();
    expect(wasm.heap().input).toBe(0);
    expect(input.live - wasm.heap().live).toBe(input.input);

    const before = wasm.heap().live;
    wasm.x.pm_heap_reset_peak();
    const len = wasm.x.pm_to_json();
    const output = wasm.heap();
    expect(len).toBe(text.length);
    expect(wasm.readOut(len)).toBe(text);
    expect(output.output).toBeGreaterThanOrEqual(len);
    // The serializer may keep its small streaming scratch buffer, but never
    // another complete document alongside the returned allocation.
    expect(output.peak - before).toBeLessThan(output.output + 1024 * 1024);

    wasm.x.pm_heap_reset_peak();
    wasm.x.pm_to_json();
    expect(wasm.heap().peak - output.live).toBeLessThan(1024 * 1024);
    const live = wasm.heap().live;
    wasm.releaseOutput();
    expect(wasm.heap().output).toBe(0);
    expect(live - wasm.heap().live).toBe(output.output);

    const bytes = wasm.toJsonBytes();
    expect(wasm.heap().output).toBe(0);
    expect(new TextDecoder().decode(bytes)).toBe(text);
    expect(wasm.toJson()).toBe(text);
    expect(wasm.heap().output).toBe(0);
    wasm.resync('{"data":"changed"}', 2);
    expect(new TextDecoder().decode(bytes)).toBe(text);
  }, 30_000);
});
