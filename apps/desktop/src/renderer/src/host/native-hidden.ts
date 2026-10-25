import { ShaderFallbackRunner } from '../engine-connection/shader-fallback';
const runner = new ShaderFallbackRunner();
const bytes = (base64: string): Uint8Array => Uint8Array.from(atob(base64), c => c.charCodeAt(0));
function base64(data: Uint8Array): string {
  let text = '';
  for (let start = 0; start < data.length; start += 8192) text += String.fromCharCode(...data.subarray(start, start + 8192));
  return btoa(text);
}
export async function runShader(request: any, inputs: string[]) {
  try {
    const float32 = request.colorDepth === 32;
    const textures = inputs.map(input => {
      const data = bytes(input);
      return float32 ? new Float32Array(data.buffer) : new Uint16Array(data.buffer);
    });
    const rendered = await runner.render(request, textures);
    const pixels = float32 ? rendered.rgba32f : rendered.rgba16f;
    if (!pixels) throw new Error('The ANGLE runner omitted float32 pixels');
    return { ok: true as const, pixels: base64(new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength)) };
  } catch (error) { return { ok: false as const, reason: String((error as any).reason ?? 'unavailable'), message: String((error as Error).message ?? error) }; }
}

// The standalone Node CLI injects this same module into its minimal ANGLE page.
(globalThis as any).runNativeShader = runShader;
