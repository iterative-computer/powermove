'use strict';
// Main-process load for the S1 gate (plan section 18): autosave serialization of a
// large document, agent-like IPC traffic, a 50 ms long task every second, and window
// resizes. Every generator records what it actually did so the report can state the
// load that was applied, not the load that was asked for.

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { performance } = require('node:perf_hooks');

/** Deterministic PRNG so every run serializes the same document. */
function mulberry32(seed) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * A synthetic project shaped like the editor's document: `layers` layers with
 * animated transforms, effects, masks and text, plus an undo history of whole-layer
 * snapshots (the store keeps history next to the project in `projectState.*`).
 * The real largest corpus project is private and stays outside the repository; this
 * stands in for it at a comparable or larger size (the byte count is reported).
 */
function buildProject({ layers = 884, keysPerProperty = 12, historyEntries = 120 } = {}) {
  const rnd = mulberry32(884);
  const keys = (base, spread) =>
    Array.from({ length: keysPerProperty }, (_, i) => ({
      t: +(i * 0.5).toFixed(3),
      v: Array.isArray(base) ? base.map(b => +(b + (rnd() - 0.5) * spread).toFixed(4)) : +(base + (rnd() - 0.5) * spread).toFixed(4),
      ease: { type: 'bezier', o: [0.33, 0], i: [0.67, 1] }
    }));
  const layer = i => ({
    id: `layer-${i.toString(36)}-${Math.floor(rnd() * 1e9).toString(36)}`,
    name: `Layer ${i}`,
    type: ['solid', 'text', 'image', 'video', 'shape', 'group'][i % 6],
    in: +(rnd() * 4).toFixed(3),
    out: +(8 + rnd() * 20).toFixed(3),
    parent: i > 10 && rnd() < 0.3 ? `layer-${Math.floor(rnd() * i).toString(36)}` : null,
    blend: ['normal', 'screen', 'multiply', 'add'][i % 4],
    transform: {
      position: { keys: keys([960, 540], 800) },
      scale: { keys: keys([1, 1], 0.5) },
      rotation: { keys: keys(0, 90) },
      opacity: { keys: keys(1, 0.4) },
      anchor: { value: [0.5, 0.5] }
    },
    effects: Array.from({ length: 1 + (i % 3) }, (_, e) => ({
      id: `fx-${i}-${e}`,
      type: ['blur', 'glow', 'colorCorrect', 'noise'][e % 4],
      enabled: true,
      params: { amount: { keys: keys(0.5, 1) }, radius: { value: +(rnd() * 40).toFixed(2) }, tint: { value: [rnd(), rnd(), rnd(), 1] } }
    })),
    masks: i % 5 === 0 ? [{ id: `m-${i}`, mode: 'add', feather: 4, path: Array.from({ length: 24 }, () => [rnd() * 1920, rnd() * 1080]) }] : [],
    text: i % 6 === 1 ? { content: `Title line ${i} `.repeat(4), font: 'DM Sans', size: 72, tracking: 0, lineHeight: 1.2, fill: [1, 1, 1, 1] } : null,
    asset: i % 6 === 2 || i % 6 === 3 ? { id: `asset-${i % 40}`, fit: 'cover', speed: 1 } : null
  });
  const project = {
    id: 'synthetic-884',
    version: 12,
    composition: { width: 3840, height: 2160, fps: { num: 60000, den: 1001 }, duration: 30, background: [0, 0, 0, 1] },
    layers: Array.from({ length: layers }, (_, i) => layer(i)),
    assets: Array.from({ length: 40 }, (_, i) => ({ id: `asset-${i}`, name: `clip-${i}.mov`, kind: 'video', width: 3840, height: 2160, duration: 12.5, hash: Math.floor(rnd() * 2 ** 52).toString(16) })),
    markers: Array.from({ length: 64 }, (_, i) => ({ t: i * 0.5, label: `m${i}` }))
  };
  const history = Array.from({ length: historyEntries }, (_, h) => ({
    label: 'Edit layer',
    at: 1_700_000_000_000 + h * 1000,
    before: layer(h % layers),
    after: layer(h % layers)
  }));
  return { project, state: { selection: [project.layers[0].id], playhead: 4.2, history } };
}

class MainLoad {
  constructor({ win, ipcMain, log, nowUs }) {
    this.win = win;
    // CLOCK_UPTIME_RAW microseconds, the clock every other S1 timestamp uses, so the
    // analysis can line load events up with presented frames.
    this.nowUs = nowUs;
    this.events = [];
    this.ipcMain = ipcMain;
    this.log = log;
    this.timers = [];
    this.records = { longTasks: [], autosaves: [], agent: { sent: 0, replies: 0, replyBytes: 0, mainUs: [] }, resizes: [] };
    this.tmp = path.join(os.tmpdir(), `s1-autosave-${process.pid}.json`);
  }

  start({ longTask = true, autosave = true, agent = true, resize = true } = {}) {
    if (autosave) {
      const doc = buildProject();
      // The renderer hands main a serialized project; main parses, clones (the store
      // keeps a structured clone), re-serializes, and writes (main/storage.ts).
      this.serialized = JSON.stringify(doc);
      this.records.documentBytes = Buffer.byteLength(this.serialized);
      this.timers.push(setInterval(() => this.autosave(), 2000));
    }
    if (longTask) {
      this.timers.push(
        setInterval(() => {
          const t0 = performance.now();
          const u0 = this.nowUs();
          while (performance.now() - t0 < 50) {
            /* the 50 ms long task */
          }
          this.records.longTasks.push([t0, performance.now() - t0]);
          this.events.push(['longTask', u0, performance.now() - t0]);
        }, 1000)
      );
    }
    if (agent) {
      // Agent tool traffic: main asks the editor renderer for tool results 20 times a
      // second (64 KB requests, 128 KB replies) and serializes each reply as the MCP
      // server would for the agent's stdio.
      const payload = { tool: 'get_layers', args: { detail: 'full', ids: Array.from({ length: 2000 }, (_, i) => `layer-${i}`) }, blob: 'x'.repeat(40_000) };
      this.onReply = (_e, reply) => {
        const t0 = performance.now();
        const text = JSON.stringify({ jsonrpc: '2.0', id: reply.id, result: reply });
        this.records.agent.replies += 1;
        this.records.agent.replyBytes += text.length;
        this.records.agent.mainUs.push((performance.now() - t0) * 1000);
      };
      this.ipcMain.on('s1:agent-reply', this.onReply);
      this.timers.push(
        setInterval(() => {
          if (this.win.isDestroyed()) return;
          this.win.webContents.send('s1:agent', { id: this.records.agent.sent++, ...payload });
        }, 50)
      );
    }
    if (resize) {
      // A one-second interactive resize every five seconds, at 60 setBounds per second.
      this.timers.push(
        setInterval(() => {
          const base = this.win.getBounds();
          let n = 0;
          const t0 = performance.now();
          const u0 = this.nowUs();
          const step = setInterval(() => {
            if (this.win.isDestroyed()) return clearInterval(step);
            const dw = Math.round(Math.min(160, base.width * 0.4) * Math.sin((n / 60) * Math.PI * 2));
            this.win.setBounds({ x: base.x, y: base.y, width: base.width + dw, height: base.height + Math.round(dw * 0.5625) });
            if (++n > 60) {
              clearInterval(step);
              this.win.setBounds(base);
              this.records.resizes.push([t0, performance.now() - t0, n]);
              this.events.push(['resize', u0, performance.now() - t0]);
            }
          }, 1000 / 60);
          this.timers.push(step);
        }, 5000)
      );
    }
  }

  autosave() {
    const t0 = performance.now();
    const u0 = this.nowUs();
    const value = JSON.parse(this.serialized);
    const cloned = structuredClone(value);
    const text = JSON.stringify(cloned);
    const sync = performance.now() - t0;
    this.events.push(['autosave', u0, sync]);
    fs.promises.writeFile(this.tmp, text).then(
      () => this.records.autosaves.push([t0, sync, performance.now() - t0]),
      err => this.log(`autosave write failed: ${err.message}`)
    );
  }

  stop() {
    for (const t of this.timers) clearInterval(t);
    this.timers = [];
    if (this.onReply) this.ipcMain.removeListener('s1:agent-reply', this.onReply);
    fs.promises.rm(this.tmp, { force: true }).catch(() => {});
    return this.summary();
  }

  summary() {
    const r = this.records;
    const stat = xs => {
      if (!xs.length) return null;
      const s = [...xs].sort((a, b) => a - b);
      return { n: s.length, p50: s[s.length >> 1], max: s[s.length - 1] };
    };
    return {
      documentBytes: r.documentBytes ?? 0,
      longTasks: { n: r.longTasks.length, ms: stat(r.longTasks.map(x => x[1])) },
      autosaves: { n: r.autosaves.length, mainThreadMs: stat(r.autosaves.map(x => x[1])), totalMs: stat(r.autosaves.map(x => x[2])) },
      agent: { requests: r.agent.sent, replies: r.agent.replies, replyBytes: r.agent.replyBytes, mainUs: stat(r.agent.mainUs) },
      resizes: { n: r.resizes.length, steps: r.resizes.reduce((a, x) => a + x[2], 0) },
      // [kind, start (CLOCK_UPTIME_RAW us), main-thread duration ms]
      events: this.events.map(([k, u, d]) => [k, Math.round(u), +d.toFixed(2)])
    };
  }
}

module.exports = { MainLoad, buildProject };
