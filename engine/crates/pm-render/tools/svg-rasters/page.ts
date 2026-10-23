// Chromium's level 1 SVG rasters drawn by the app's own module
// (apps/desktop/src/renderer/src/engine-connection/svg-rasters.ts, decision
// S1) in a hidden Electron window: the jobs come from run.ts, the SVG bytes
// from its server, and the key's canvas-bucket or material-map bytes go back.
import { SvgRasterCanvas, decodeSvg } from '../../../../../apps/desktop/src/renderer/src/engine-connection/svg-rasters';

type Job = { texture?: boolean; id: string; width: number; height: number };

async function post(path: string, body: BodyInit, params: Record<string, string | number> = {}) {
  const q = new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)]));
  const r = await fetch(`/${path}?${q}`, { method: 'POST', body });
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
}

(async () => {
  try {
    const jobs: Job[] = await (await fetch('/jobs')).json();
    const canvas = new SvgRasterCanvas();
    // One decode per document, as an asset's <img> serves every bucket.
    const decoded = new Map<string, Promise<HTMLImageElement>>();
    for (const job of jobs) {
      try {
        const source = new URL(`/svg?id=${encodeURIComponent(job.id)}`, location.href).toString();
        let img = decoded.get(source);
        if (!img) {
          img = fetch(source).then(async (r) => decodeSvg(new Uint8Array(await r.arrayBuffer())));
          decoded.set(source, img);
        }
        const px = job.texture ? canvas.drawTexture(await img, job.width, job.height) : canvas.draw(await img, job.width, job.height);
        await post('raster', px as BodyInit, { id: job.id, w: job.width, h: job.height });
      } catch (e) {
        await post('raster', String((e as Error)?.message || e), { id: job.id, error: 1 });
      }
    }
    await post('done', String(jobs.length));
  } catch (e) {
    await post('failed', String((e as Error)?.stack || e));
  }
})();
