import type { ChildProcess } from 'node:child_process';

/** macOS can stall Electron in native teardown after Node's successful exit
 * event (electron/electron#52582). Never reap a live/failed/cancelled editor:
 * the private marker is written synchronously at that final successful event. */
export function watchCompletedExit(child: ChildProcess, marker: string, onReap: () => void, grace = 3000): () => void {
  let tail = '', timer: ReturnType<typeof setTimeout> | undefined;
  const cleanup = () => {
    clearTimeout(timer);
    child.stderr?.off('data', receive);
    child.off('exit', cleanup);
  };
  const receive = (data: Buffer | string) => {
    const received = tail + String(data);
    tail = received.slice(-marker.length * 2);
    if (timer || !received.includes(marker)) return;
    timer = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) {
        onReap(); child.kill('SIGKILL');
      }
      cleanup();
    }, grace);
  };
  child.stderr?.on('data', receive);
  child.once('exit', cleanup);
  return cleanup;
}
