/** Let input and paint run without depending on timers that are throttled
 * when a desktop window is hidden or covered by another application. */
export function yieldTask(): Promise<void> {
  const scheduler = (globalThis as typeof globalThis & { scheduler?: { yield?(): Promise<void> } }).scheduler;
  if (typeof scheduler?.yield === 'function') return scheduler.yield();
  return yieldTurn();
}

/** Like yieldTask, but behind the tasks already waiting: `scheduler.yield()`
 *  resumes ahead of them, so a long run of its slices holds back replies
 *  that other work of the same operation waits for. */
export function yieldTurn(): Promise<void> {
  if (typeof MessageChannel === 'undefined') return new Promise(resolve => setTimeout(resolve, 0));
  return new Promise(resolve => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => { channel.port1.close(); channel.port2.close(); resolve(); };
    channel.port2.postMessage(null);
  });
}
