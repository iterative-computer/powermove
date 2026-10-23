// @vitest-environment happy-dom
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, it, vi } from 'vitest';
import { importFileInPage } from './page-import';

it.each([undefined, 0, 0.5])('uses the editor importer with placement %s and its default options', async at => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-page-import-'));
  try {
    const file = path.join(root, 'mark.svg'); await writeFile(file, '<svg/>');
    const PM = { proj: { id: 'P', assets: {} as Record<string, unknown>, layers: [] },
      importFiles: vi.fn(async () => { PM.proj.assets.a1 = { id: 'a1', name: 'mark.svg', kind: 'image' }; }) };
    (window as any).PM = PM;
    const page = {
      evaluate: (fn: (value: any) => unknown, value?: unknown) => fn(value),
      locator: () => ({
        setInputFiles: async () => Object.defineProperty(document.querySelector('input'), 'files', { value: [new File(['<svg/>'], 'mark.svg', { type: 'image/svg+xml' })] }),
        evaluate: async (fn: (element: Element) => unknown) => fn(document.querySelector('input')!)
      })
    };
    expect(await importFileInPage(page as never, { path: file, projectId: 'P', ...(at === undefined ? {} : { at }) })).toEqual({
      assets: [{ id: 'a1', name: 'mark.svg', kind: 'image' }], layers: []
    });
    expect(PM.importFiles).toHaveBeenCalledExactlyOnceWith(expect.any(Array), { placement: at === undefined ? null : { at } });
    expect(document.querySelector('input')).toBeNull();
  } finally { delete (window as any).PM; await rm(root, { recursive: true, force: true }); }
});
