import { BlenderPngs } from '../../../../../apps/desktop/src/renderer/src/engine-connection/blender-png';

const post = (path: string, id: string, data: BodyInit) =>
  fetch(`/${path}?id=${encodeURIComponent(id)}`, { method: 'POST', body: data });

(async () => {
  const ids: string[] = await (await fetch('/needs')).json();
  for (const id of ids) {
    try {
      const source = await fetch(`/source?id=${encodeURIComponent(id)}`);
      if (!source.ok) throw new Error(`source: ${source.status}`);
      const blob = await source.blob();
      const image = blob.type === 'image/svg+xml' ? await new Promise<HTMLImageElement>((resolve, reject) => {
        const url = URL.createObjectURL(blob), img = new Image();
        img.onload = () => { URL.revokeObjectURL(url); resolve(img); };
        img.onerror = () => { URL.revokeObjectURL(url); reject(new Error('SVG image decode failed')); };
        img.src = url;
      }) : null;
      const converter = new BlenderPngs({
        meta: () => ({ kind: 'image', name: id }),
        blob: async () => blob,
        image: () => image ? { el: image } : null,
        canvas: () => document.createElement('canvas'),
        decode: blob => createImageBitmap(blob),
      });
      const hash = new Uint8Array((id.match(/../g) ?? []).map(h => parseInt(h, 16)));
      const answer = await converter.produce({ document: 'corpus', composition: 'blender', layer: 'blender',
        time: 0, source: 'blenderPng', scale: 1, revision: 0,
        blender: { asset: id, content: hash } });
      await post('png', id, answer.data as BodyInit);
    } catch (error) {
      await post('error', id, String((error as Error).stack ?? error));
    }
  }
  await post('done', '', String(ids.length));
})().catch(error => post('error', '', String(error)));
