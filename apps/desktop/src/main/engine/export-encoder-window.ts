/*
 * The hidden export encoder window (export-encoder.ts): a sandboxed page
 * of our own that runs today's WebCodecs WebM encoding. It is never shown
 * or focused, is not an editor window, and talks to main only over the
 * MessagePort its preload hands it.
 */
import { BrowserWindow, MessageChannelMain } from 'electron';

import { EXPORT_ENCODER_PORT, type EncoderReply } from '../../shared/export-encoder';
import type { EncoderWindow } from './export-encoder';

/** How long the page may take to load and answer `ready`. */
const READY_MS = 30_000;

export function openEncoderWindow(options: { preload: string; url: string; log?: (level: 'info' | 'warn', message: string) => void; receiverGone?: (owner: number) => void }): Promise<EncoderWindow> {
  const window = new BrowserWindow({
    show: false,
    width: 64,
    height: 64,
    skipTaskbar: true,
    focusable: false,
    webPreferences: {
      preload: options.preload,
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      // Encoding continues while every window is in the background.
      backgroundThrottling: false,
      spellcheck: false,
      devTools: false
    }
  });
  const { port1, port2 } = new MessageChannelMain();
  const replies: Array<(reply: EncoderReply) => void> = [];
  const gone: Array<(reason: string) => void> = [];
  let ended = false;
  const owner = window.webContents.id;
  const receiverGone = () => options.receiverGone?.(owner);
  window.webContents.on('did-start-navigation', (details) => { if (details.isMainFrame && !details.isSameDocument) receiverGone(); });
  window.webContents.on('render-process-gone', receiverGone);
  window.webContents.on('destroyed', receiverGone);
  const end = (reason: string) => {
    if (ended) return;
    ended = true;
    for (const listener of gone) listener(reason);
  };
  // The page reports nothing on success; its warnings and errors go to main's log.
  window.webContents.on('console-message', (event) => {
    if (event.level === 'warning' || event.level === 'error') options.log?.('warn', `[export encoder] ${event.message}`);
  });
  window.webContents.on('render-process-gone', (_event, details) => end(`the encoder process ${details.reason}`));
  window.on('closed', () => end('the encoder window closed'));
  const encoder: EncoderWindow = {
    post: (command) => port1.postMessage(command),
    get receiver() { return { frame: window.webContents.mainFrame, owner: window.webContents.id }; },
    retire: (generation) => { if (!window.isDestroyed()) window.webContents.send('export-encoder:retire', generation); },
    onReply: (listener) => { replies.push(listener); },
    onGone: (listener) => { gone.push(listener); },
    close: () => {
      ended = true;
      port1.close();
      if (!window.isDestroyed()) window.destroy();
    }
  };
  return new Promise<EncoderWindow>((resolve, reject) => {
    let settled = false;
    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      encoder.close();
      reject(error);
    };
    const timer = setTimeout(() => fail(new Error('The export encoder did not start.')), READY_MS);
    gone.push((reason) => fail(new Error(`The export encoder did not start: ${reason}.`)));
    port1.on('message', (event) => {
      const reply = event.data as EncoderReply;
      if (reply?.type === 'ready' && !settled) {
        settled = true;
        clearTimeout(timer);
        resolve(encoder);
        return;
      }
      for (const listener of replies) listener(reply);
    });
    port1.start();
    window.loadURL(options.url).then(() => {
      if (!window.isDestroyed()) window.webContents.postMessage(EXPORT_ENCODER_PORT, null, [port2]);
    }, (error: unknown) => fail(error instanceof Error ? error : new Error(String(error))));
  });
}
