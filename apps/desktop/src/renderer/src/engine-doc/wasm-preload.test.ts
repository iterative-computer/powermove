import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });
const emptyModule = new Uint8Array([0,97,115,109,1,0,0,0]);

it('consumes the preload setting/module once and reuses its timing without another fetch', async () => {
  vi.resetModules();
  const module = await WebAssembly.compile(emptyModule);
  const timing = {sourceMs:2,compileMs:11};
  vi.stubGlobal('__powermoveDocumentBoot',{enabled:true,module:Promise.resolve(module),timing});
  const fetch = vi.fn(); vi.stubGlobal('fetch',fetch);
  const wasm = await import('./wasm');
  expect((globalThis as any).__powermoveDocumentBoot).toBeUndefined();
  expect(wasm.preloadedEngineEnabled()).toBe(true);
  const pending = wasm.compileEditorModule();
  expect(wasm.compileEditorModule()).toBe(pending);
  expect(await pending).toBe(module);
  expect(wasm.moduleCompileTiming()).toEqual(timing);
  expect(fetch).not.toHaveBeenCalled();
});

it('allows a real retry after an early preload failure', async () => {
  vi.resetModules();
  const failed = Promise.reject(new Error('early fetch failed')); void failed.catch(()=>{});
  vi.stubGlobal('__powermoveDocumentBoot',{enabled:true,module:failed});
  vi.stubGlobal('location',{href:'https://editor.invalid/'});
  const fetch = vi.fn(async () => new Response(emptyModule,{headers:{'content-type':'application/octet-stream'}}));
  vi.stubGlobal('fetch',fetch);
  const wasm = await import('./wasm');
  await expect(wasm.compileEditorModule()).rejects.toThrow('early fetch failed');
  expect(await wasm.compileEditorModule()).toBeInstanceOf(WebAssembly.Module);
  expect(fetch).toHaveBeenCalledTimes(1);
});
