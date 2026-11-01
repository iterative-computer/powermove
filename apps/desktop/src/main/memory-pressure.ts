import { execFile } from 'node:child_process';
import { BrowserWindow } from 'electron';

/*
 * macOS reports system memory pressure through the kernel. Chromium purges
 * its own caches on that signal, but the editor's caches (decoded audio,
 * rendered preview frames, live video decoders) only shrink when the renderer
 * hears about it. Poll the level cheaply and forward changes as the
 * `memorypressure` event the editor's memory registry already handles.
 */
type Level = 'normal' | 'moderate' | 'critical';

export function levelFromSysctl(output: string): Level {
  const value = Number(String(output).trim());
  if (value >= 4) return 'critical';
  if (value >= 2) return 'moderate';
  return 'normal';
}

function readLevel(): Promise<Level> {
  return new Promise(resolve => {
    execFile('/usr/sbin/sysctl', ['-n', 'kern.memorystatus_vm_pressure_level'], { timeout: 2000 }, (error, stdout) => {
      resolve(error ? 'normal' : levelFromSysctl(stdout));
    });
  });
}

export function watchMemoryPressure(intervalMs = 10_000): () => void {
  if (process.platform !== 'darwin') return () => {};
  let last: Level = 'normal', stopped = false, busy = false;
  const notify = (level: Exclude<Level, 'normal'>) => {
    for (const window of BrowserWindow.getAllWindows()) {
      const contents = window.webContents;
      if (contents.isDestroyed()) continue;
      contents.executeJavaScript(`window.dispatchEvent(new CustomEvent('memorypressure', { detail: ${JSON.stringify(level)} }))`, false)
        .catch(() => undefined);
    }
  };
  const poll = async () => {
    if (busy || stopped) return;
    busy = true;
    try {
      const level = await readLevel();
      // Repeat critical while it lasts; report moderate once per episode.
      if (level === 'critical' || (level === 'moderate' && last === 'normal')) notify(level);
      last = level;
    } finally { busy = false; }
  };
  const timer = setInterval(() => { void poll(); }, intervalMs);
  timer.unref?.();
  return () => { stopped = true; clearInterval(timer); };
}
