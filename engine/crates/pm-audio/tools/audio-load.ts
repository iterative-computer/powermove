/** Hidden real-app audio cost: paused, playing, and playing under CPU load.
 * Run under pm-electron-lock:
 * bun engine/crates/pm-audio/tools/audio-load.ts <wav> <outside-repo.json> [seconds]
 * App and native addons must already be built (see play-start.ts).
 * Per window it records the Electron process tree's CPU time (ps), its idle
 * wakeups and energy impact (top's second sample, same interval), coreaudiod's
 * CPU, and for the native engine the device's callback count, CoreAudio
 * processor overloads and ring underruns. PM_AUDIO_START_MODE=today|native
 * limits the run to one engine. PM_AUDIO_LOAD_THREADS (default 4) busy loops
 * run during the stressed window.
 */
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { launchLegacyApp } from '../../../corpus/src/app';

const [file, output, secondsArg = '15'] = process.argv.slice(2);
if (!file || !output) throw new Error('wav and outside-repository output required');
const seconds = Number(secondsArg);
const loadThreads = Number(process.env.PM_AUDIO_LOAD_THREADS ?? 4);
process.env.POWERMOVE_PM_BRIDGE ??= resolve(import.meta.dir, '../../../../apps/desktop/native/pm_bridge.node');

interface Proc { pid: number; ppid: number; cpu: number; command: string }

async function run(cmd: string[]): Promise<string> {
  const p = Bun.spawn(cmd, { stdout: 'pipe', stderr: 'ignore' });
  const text = await new Response(p.stdout).text();
  await p.exited;
  return text;
}

/** CPU seconds from ps's [[dd-]hh:]mm:ss.ss. */
function cpuSeconds(t: string): number {
  const [days, rest] = t.includes('-') ? t.split('-') : ['0', t];
  return rest.split(':').reduce((acc, part) => acc * 60 + Number(part), 0) + Number(days) * 86400;
}

async function processes(): Promise<Proc[]> {
  const text = await run(['ps', '-A', '-o', 'pid=,ppid=,time=,comm=']);
  return text.trim().split('\n').map(line => {
    const [pid, ppid, time, ...command] = line.trim().split(/\s+/);
    return { pid: Number(pid), ppid: Number(ppid), cpu: cpuSeconds(time), command: command.join(' ') };
  });
}

function tree(all: Proc[], root: number): Set<number> {
  const set = new Set([root]);
  for (let grew = true; grew;) {
    grew = false;
    for (const p of all) if (!set.has(p.pid) && set.has(p.ppid)) { set.add(p.pid); grew = true; }
  }
  return set;
}

/** top's interval sample (the second of two) for every process. */
async function topSample(interval: number): Promise<Map<number, { cpu: number; idlew: number; power: number }>> {
  const text = await run(['top', '-l', '2', '-s', String(interval), '-stats', 'pid,cpu,idlew,power']);
  const lastHeader = text.lastIndexOf('PID');
  const rows = new Map<number, { cpu: number; idlew: number; power: number }>();
  for (const line of text.slice(lastHeader).split('\n').slice(1)) {
    const [pid, cpu, idlew, power] = line.trim().split(/\s+/);
    if (!pid || Number.isNaN(Number(pid))) continue;
    rows.set(Number(pid), { cpu: parseFloat(cpu), idlew: parseFloat(idlew), power: parseFloat(power) });
  }
  return rows;
}

const result: Record<string, unknown> = {
  method: 'hidden isolated app; ps CPU-time delta of the Electron process tree; top interval sample for idle wakeups and energy impact',
  seconds, loadThreads,
};
const modes = process.env.PM_AUDIO_START_MODE === 'today' ? [false] : process.env.PM_AUDIO_START_MODE === 'native' ? [true] : [false, true];
for (const native of modes) {
  process.env.POWERMOVE_NEW_ENGINE = native ? '1' : '0';
  const session = await launchLegacyApp();
  console.error(`audio load: ${native ? 'native' : 'today'} launched`);
  const stress: Bun.Subprocess[] = [];
  try {
    await session.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Audio load measurement', w: 160, h: 90, fps: 30, dur: 10 }));
      PM.loop = true;
      const start = AudioBufferSourceNode.prototype.start;
      AudioBufferSourceNode.prototype.start = function (...args: Parameters<typeof start>) {
        (window as any).__loadCtx = this.context;
        return start.apply(this, args);
      };
      const input = document.createElement('input');
      input.type = 'file'; input.id = 'load-media'; input.style.display = 'none';
      document.body.appendChild(input);
    });
    await session.setInputFiles('#load-media', [file]);
    await session.evaluate(async () => {
      const PM = (window as any).PM;
      const input = document.getElementById('load-media') as HTMLInputElement;
      const asset = await PM.assets.add(input.files![0], { newAsset: true, silent: true });
      PM.proj.layers.push(PM.mkLayer('audio', { from: 0, dur: 10, d: { asset: asset.id, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 } }, PM.proj));
      PM.touch();
      PM.bus.emit('layers'); PM.bus.emit('project');
      PM.invalidate();
    });
    if (native) {
      await session.waitFor(() => (window as any).PM.EngineConnection.status().driving);
    } else {
      await session.waitFor(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.audioBuffer));
    }
    // Open the device as a user's first play does, then settle paused.
    await session.evaluate(() => (window as any).PM.play());
    await Bun.sleep(500);
    await session.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.setTime(0); });
    await Bun.sleep(3000);
    const electronPid = await session.evaluateMain(() => process.pid);
    const audioStatus = async () => native ? await session.evaluateMain(async () => {
      const report = await (globalThis as any).__powermoveNativeEngine.engine.report('interactive');
      return Object.values(report.presenter.audio)[0] as any;
    }) : await session.evaluate(() => {
      // Chromium exposes no glitch counter unless AudioContext.playbackStats ships.
      const ctx = (window as any).__loadCtx;
      return { contextState: ctx?.state, baseLatency: ctx?.baseLatency, playbackStats: ctx && 'playbackStats' in ctx ? JSON.parse(JSON.stringify(ctx.playbackStats)) : null };
    });
    const windows: Record<string, unknown> = {};
    for (const phase of ['paused', 'playing', 'playingUnderLoad'] as const) {
      if (phase !== 'paused') {
        await session.evaluate(() => { const PM = (window as any).PM; if (!PM.playing) PM.play(); });
        await Bun.sleep(1000);
      }
      if (phase === 'playingUnderLoad') {
        for (let i = 0; i < loadThreads; i++) stress.push(Bun.spawn(['/bin/sh', '-c', 'while :; do :; done']));
        await Bun.sleep(500);
      }
      const before = await processes();
      const statusBefore = await audioStatus();
      const t0 = performance.now();
      const top = await topSample(seconds);
      const after = await processes();
      const elapsed = (performance.now() - t0) / 1000;
      const statusAfter = await audioStatus();
      const members = tree(after, electronPid);
      const cpu = (list: Proc[], ids: Set<number>) => list.filter(p => ids.has(p.pid)).reduce((a, p) => a + p.cpu, 0);
      const coreaudiod = new Set(after.filter(p => p.command.endsWith('coreaudiod')).map(p => p.pid));
      const byProcess = [...members].map(pid => {
        const a = after.find(p => p.pid === pid)!, b = before.find(p => p.pid === pid);
        return { pid, command: a.command.split('/').at(-1), cpuPercent: 100 * (a.cpu - (b?.cpu ?? 0)) / elapsed, top: top.get(pid) };
      }).filter(p => p.cpuPercent > 0.05 || (p.top?.idlew ?? 0) > 0);
      const topSum = (key: 'idlew' | 'power') => [...members].reduce((a, pid) => a + (top.get(pid)?.[key] ?? 0), 0);
      windows[phase] = {
        elapsedS: elapsed,
        treeCpuPercent: 100 * (cpu(after, members) - cpu(before, members)) / elapsed,
        treeIdleWakeups: topSum('idlew'),
        treeEnergyImpact: topSum('power'),
        coreaudiodCpuPercent: 100 * (cpu(after, coreaudiod) - cpu(before, coreaudiod)) / elapsed,
        byProcess,
        audio: native ? {
          callbacks: statusAfter.deviceCallbacks - statusBefore.deviceCallbacks,
          deviceOverloads: statusAfter.deviceOverloads - statusBefore.deviceOverloads,
          ringUnderruns: statusAfter.underruns - statusBefore.underruns,
          starvedRenders: statusAfter.starvedRenders - statusBefore.starvedRenders,
          problems: statusAfter.problems,
          device: statusAfter.device, deviceRate: statusAfter.deviceRate, latencyUs: statusAfter.latencyUs,
        } : { before: statusBefore, after: statusAfter },
      };
      if (phase === 'playingUnderLoad') { for (const s of stress.splice(0)) s.kill(); }
      console.error(`audio load: ${native ? 'native' : 'today'} ${phase} done`);
    }
    await session.evaluate(() => (window as any).PM.pause());
    result[native ? 'native' : 'today'] = { windows, pageErrors: session.pageErrors };
    await writeFile(output, JSON.stringify(result, null, 2));
  } catch (error) {
    result.failure = { engine: native ? 'native' : 'today', error: String(error), pageErrors: session.pageErrors, consoleErrors: session.consoleErrors };
    await writeFile(output, JSON.stringify(result, null, 2));
    throw error;
  } finally {
    for (const s of stress) s.kill();
    await session.close();
  }
}
console.log(JSON.stringify(result, null, 2));
