import { ShaderFallbackRunner } from '../engine-connection/shader-fallback';
const runner = new ShaderFallbackRunner();
const bytes = (base64: string): Uint8Array => Uint8Array.from(atob(base64), c => c.charCodeAt(0));
export async function runShader(request: any, inputs: string[]) {
  try {
    const textures = inputs.map(input => new Uint16Array(bytes(input).buffer));
    const rendered = await runner.render(request, textures);
    const data = new Uint8Array(rendered.rgba16f.buffer, rendered.rgba16f.byteOffset, rendered.rgba16f.byteLength);
    let text = '';
    for (let start = 0; start < data.length; start += 8192) text += String.fromCharCode(...data.subarray(start, start + 8192));
    return { ok: true as const, pixels: btoa(text) };
  } catch (error) { return { ok: false as const, reason: String((error as any).reason ?? 'unavailable'), message: String((error as Error).message ?? error) }; }
}
// The standalone Node CLI injects this same module into its minimal ANGLE page.
(globalThis as any).runNativeShader = runShader;
