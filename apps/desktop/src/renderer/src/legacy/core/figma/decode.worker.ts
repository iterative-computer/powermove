import { decodeFigmaClipboard } from './clipboard';

self.onmessage = async (event: MessageEvent<string>) => {
  try { self.postMessage({ selection: await decodeFigmaClipboard(event.data) }); }
  catch (error) { self.postMessage({ error: error instanceof Error ? error.message : 'Could not read the copied Figma layers.' }); }
};
