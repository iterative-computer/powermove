// Tessellates STEP, IGES and BREP files with OpenCascade (occt-import-js,
// LGPL-2.1, loaded only here) off the main thread.
import occtimportjs from 'occt-import-js';
import wasmUrl from 'occt-import-js/dist/occt-import-js.wasm?url';

let occt: Promise<any> | null = null;

self.onmessage = async (e: MessageEvent<{ format: 'step' | 'iges' | 'brep'; bytes: Uint8Array; params: Record<string, unknown> }>) => {
  try {
    occt ??= occtimportjs({ locateFile: () => wasmUrl });
    const result = (await occt).ReadFile(e.data.format, e.data.bytes, e.data.params);
    if (!result?.success) throw new Error('The CAD file could not be read');
    // Flat typed arrays, transferred instead of copied.
    const transfer: ArrayBuffer[] = [];
    const meshes = result.meshes.map((m: any) => {
      const position = new Float32Array(m.attributes.position.array);
      const normal = m.attributes.normal ? new Float32Array(m.attributes.normal.array) : null;
      const index = new Uint32Array(m.index.array);
      transfer.push(position.buffer, index.buffer);
      if (normal) transfer.push(normal.buffer);
      return { name: m.name, color: m.color ?? null, faces: m.brep_faces ?? [], position, normal, index };
    });
    (self as any).postMessage({ ok: true, root: result.root, meshes }, transfer);
  } catch (error) {
    (self as any).postMessage({ ok: false, message: error instanceof Error ? error.message : String(error) });
  }
};
