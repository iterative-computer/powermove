import { expect, test } from './helpers/app';

test('retained value, speed, and merged graphs match complete paints with live curve readouts', async ({ session }) => {
  await session.openEditor();
  const results = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    const project = PM.mkProject({ name: 'Retained graph artwork', dur: 12 });
    project.layers = Array.from({ length: 200 }, (_, index) => PM.mkLayer('solid', { name: `Layer ${index}`, dur: 12 }, project));
    const layer = project.layers[0];
    for (const path of ['opacity', 'position.x', 'position.y']) {
      const prop = PM.findProp(layer, path);
      prop.kf = [[0, 25], [2, 100], [4, 50]].map(([time, value]) => PM.KF(time, value));
    }
    PM.replaceProject(project); PM.selectLayers(layer.id);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.setMode('layers'); timeline.graph = true; timeline.graphCombined = true;
    timeline.pps = 90; timeline.scrollT = timeline.scrollY = 0;
    const now = performance.now(), originalNow = performance.now;
    const originalText = CanvasRenderingContext2D.prototype.fillText;
    const originalPath = CanvasRenderingContext2D.prototype.beginPath;
    let text = 0, paths = 0;
    performance.now = () => now;
    CanvasRenderingContext2D.prototype.fillText = function (...args: Parameters<typeof originalText>) {
      if (this.canvas === timeline.cv) text++;
      return originalText.apply(this, args);
    };
    CanvasRenderingContext2D.prototype.beginPath = function () {
      if (this.canvas === timeline.cv) paths++;
      return originalPath.call(this);
    };
    const snapshot = () => {
      const canvas = document.createElement('canvas');
      canvas.width = timeline.cv.width; canvas.height = timeline.cv.height;
      const context = canvas.getContext('2d', { willReadFrequently: true })!;
      context.drawImage(timeline.cv, 0, 0);
      return { canvas, context };
    };
    const results: Array<{ mode: string; reusedHandles: boolean; maxDelta: number; changedChannels: number; cachedText: number; cachedPaths: number; fullText: number; fullPaths: number }> = [];
    try {
      for (const mode of ['value', 'speed', 'merged']) {
        const merged = mode === 'merged';
        timeline.graphType = mode === 'speed' ? 'speed' : 'value'; timeline.graphMerge = merged;
        timeline.graphFocus = { layerId: layer.id, trackKey: merged ? 'position.y' : 'opacity' };
        timeline.graphShown = [];
        PM.sel.chan = merged ? 'position.y' : 'opacity';
        PM.sel.keys = (merged ? ['position.x', 'position.y'] : ['opacity'])
          .flatMap(path => layer.p[path].kf.map((key: any) => key.i));
        PM.bus.emit('sel'); PM.bus.emit('layers');
        PM.time = .25; PM.bus.emit('time', PM.time); PM.bus.emit('draw:timeline');
        const graph = timeline._graph;
        text = paths = 0;
        PM.time = .75; PM.bus.emit('time', PM.time); PM.bus.emit('draw:timeline');
        const cachedText = text, cachedPaths = paths, reusedHandles = timeline._graph === graph;
        const cachedSnapshot = snapshot();
        PM.bus.emit('layers'); text = paths = 0; PM.bus.emit('draw:timeline');
        const fullText = text, fullPaths = paths, fullSnapshot = snapshot();
        const cached = cachedSnapshot.context.getImageData(0, 0, timeline.cv.width, timeline.cv.height).data;
        const full = fullSnapshot.context.getImageData(0, 0, timeline.cv.width, timeline.cv.height).data;
        let maxDelta = 0, changedChannels = 0;
        for (let index = 0; index < full.length; index++) {
          const delta = Math.abs(full[index] - cached[index]);
          if (delta) changedChannels++;
          maxDelta = Math.max(maxDelta, delta);
        }
        results.push({ mode, reusedHandles, maxDelta, changedChannels, cachedText, cachedPaths, fullText, fullPaths });
        cachedSnapshot.canvas.width = fullSnapshot.canvas.width = 0;
      }
    } finally {
      performance.now = originalNow;
      CanvasRenderingContext2D.prototype.fillText = originalText;
      CanvasRenderingContext2D.prototype.beginPath = originalPath;
    }
    return results;
  });
  console.log('GRAPH_PAINT_CACHE', JSON.stringify(results));
  for (const result of results) {
    expect(result.reusedHandles).toBe(true);
    expect(result.maxDelta).toBeLessThanOrEqual(1);
    expect(result.cachedPaths).toBeLessThan(result.fullPaths / 5);
    expect(result.cachedText).toBeLessThan(result.fullText / 3);
  }
  expect(results.find(result => result.mode === 'value')!.cachedText).toBeGreaterThan(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
  expect(session.diagnostics.console.filter(record => record.text.includes('[timeline draw]'))).toEqual([]);
});
