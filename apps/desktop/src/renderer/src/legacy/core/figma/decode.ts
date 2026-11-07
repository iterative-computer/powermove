import type { FigmaDoc } from '../../../../../shared/figma/doc';

/** Keep large or malformed private-format payloads off the editor thread. */
export function readFigmaDoc(html: string): Promise<FigmaDoc> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./decode.worker.ts', import.meta.url), { type: 'module' });
    const finish = () => {
      clearTimeout(timer);
      worker.terminate();
    };
    const timer = setTimeout(() => {
      finish();
      reject(new Error('Reading Figma layers took too long. Copy fewer layers and try again.'));
    }, 15000);
    worker.onmessage = ({ data }) => {
      finish();
      if (data.error) reject(new Error(data.error));
      else resolve(data.doc);
    };
    worker.onerror = () => {
      finish();
      reject(new Error('Could not read the copied Figma layers.'));
    };
    worker.postMessage(html);
  });
}
