import { describe, expect, it } from 'vitest';
import {
  LIGHT_DEFINITION, coneSilhouette, collectLightMarkers, drawLightMarkers, edgePlacement, glyphColor,
  markerAt, markerTitle, onStage, type LightMarker,
} from './light-markers';

const stage = { width: 800, height: 500 };
const rect = { x: 100, y: 50, width: 600, height: 400 };

function marker(patch: Partial<LightMarker> = {}): LightMarker {
  return { id: 'a', name: 'Point light', type: 'point', color: '#FFFFFF', intensity: 3, selected: false, locked: false,
    x: 100, y: 100, edge: null, behind: false, apex: { x: 100, y: 100 }, heading: null, guides: null, ...patch };
}

function fakeApi(lights: Record<string, any>, extra: { selected?: string[]; inactive?: string[] } = {}) {
  const layers = [{ id: 'model', d: { definition: 'powermove.3d.object' } },
    ...Object.keys(lights).map(id => ({ id, name: `Light ${id}`, d: { definition: LIGHT_DEFINITION } }))];
  return {
    model: { curComp: () => ({ layers }) },
    selection: { layers: () => extra.selected ?? [] },
    anim: { active: (layer: any) => !(extra.inactive ?? []).includes(layer.id) },
    scene3d: {
      lightViewerState: (id: string) => lights[id]?.state ?? null,
      lightViewerGuides: (id: string) => lights[id]?.guides ?? null,
    },
  } as any;
}

const state = (patch: any = {}) => ({ layerId: 'x', worldPosition: { x: 0, y: 0, z: 0 }, viewerPosition: { x: 400, y: 250 },
  inFront: true, visible: true, inFrame: true, color: '#FFB46B', intensity: 20, type: 'point', direction: null, coneAngleDegrees: null, ...patch });

describe('edge placement', () => {
  it('pins an off-stage point to the stage edge, pointing toward it', () => {
    const pin = edgePlacement({ x: 2000, y: 250 }, false, stage, 18);
    expect(pin).toMatchObject({ x: 782, y: 250 });
    expect(pin.angle).toBeCloseTo(0, 9);
    const corner = edgePlacement({ x: -1000, y: -1000 }, false, stage, 18);
    expect(corner.x).toBeGreaterThanOrEqual(18 - 1e-9); expect(corner.y).toBeCloseTo(18, 9);
  });

  it('mirrors a behind-camera projection so the chevron points the right way', () => {
    // Perspective division flips points behind the camera through the centre.
    const pin = edgePlacement({ x: 600, y: 250 }, true, stage, 18);
    expect(pin.x).toBeCloseTo(18, 9); expect(Math.abs(pin.angle)).toBeCloseTo(Math.PI, 9);
  });

  it('treats the stage margin as on-stage', () => {
    expect(onStage({ x: 3, y: 3 }, stage)).toBe(true);
    expect(onStage({ x: -1, y: 3 }, stage)).toBe(false);
    expect(onStage({ x: 799, y: 250 }, stage)).toBe(false);
  });
});

describe('cone silhouette', () => {
  const circle = (cx: number, cy: number, r: number) => Array.from({ length: 48 }, (_, i) => ({
    x: cx + Math.cos(i / 48 * Math.PI * 2) * r, y: cy + Math.sin(i / 48 * Math.PI * 2) * r }));

  it('finds the two tangent rim points seen from the apex', () => {
    const [a, b] = coneSilhouette({ x: 0, y: 0 }, circle(100, 0, 30))!;
    expect(a.x).toBeGreaterThan(85); expect(b.x).toBeGreaterThan(85);
    expect(Math.sign(a.y)).not.toBe(Math.sign(b.y));
  });

  it('has no silhouette when looking down the cone or with too few rim points', () => {
    expect(coneSilhouette({ x: 100, y: 0 }, circle(100, 0, 30))).toBeNull();
    expect(coneSilhouette({ x: 0, y: 0 }, [{ x: 1, y: 1 }, null, null])).toBeNull();
  });
});

describe('hit testing', () => {
  it('prefers the selected marker and skips locked ones', () => {
    const markers = [marker({ id: 'a', x: 100, y: 100 }), marker({ id: 'b', x: 104, y: 100, selected: true }), marker({ id: 'c', x: 300, y: 300, locked: true })];
    expect(markerAt(markers, 100, 100)?.id).toBe('b');
    expect(markerAt(markers, 300, 300)).toBeNull();
    expect(markerAt(markers, 160, 160)).toBeNull();
  });
});

describe('collecting markers', () => {
  it('shows every active light; selected ones paint last', () => {
    const api = fakeApi({ a: { state: state() }, b: { state: state({ viewerPosition: { x: 200, y: 120 }, type: 'spot' }), guides: { aim: { x: 400, y: 250 }, rim: [], innerRim: null, range: null } } }, { selected: ['a'] });
    const markers = collectLightMarkers(api, rect, stage, 0, { isLocked: () => false });
    expect(markers.map(m => m.id)).toEqual(['b', 'a']);
    expect(markers[1]).toMatchObject({ x: 400, y: 250, edge: null, selected: true, color: '#FFB46B' });
    expect(markers[0]!.heading).toBeCloseTo(Math.atan2(130, 200), 9);
  });

  it('pins off-stage and behind-camera lights instead of dropping them, and skips inactive ones', () => {
    const api = fakeApi({
      off: { state: state({ viewerPosition: { x: 1400, y: 250 }, inFrame: false }) },
      behind: { state: state({ viewerPosition: { x: 500, y: 250 }, inFront: false, visible: false, inFrame: false }) },
      hidden: { state: state() },
    }, { inactive: ['hidden'] });
    const markers = collectLightMarkers(api, rect, stage, 0, { isLocked: () => false });
    expect(markers.map(m => m.id)).toEqual(['off', 'behind']);
    expect(markers[0]).toMatchObject({ edge: 0, behind: false, apex: { x: 1400, y: 250 } });
    expect(markers[1]).toMatchObject({ behind: true, apex: null, guides: null });
    expect(Math.abs(markers[1]!.edge!)).toBeCloseTo(Math.PI, 9);
  });

  it('returns nothing without lights or a light API', () => {
    expect(collectLightMarkers(fakeApi({}), rect, stage, 0, { isLocked: () => false })).toEqual([]);
    expect(collectLightMarkers({ ...fakeApi({ a: { state: state() } }), scene3d: undefined }, rect, stage, 0, { isLocked: () => false })).toEqual([]);
  });
});

describe('appearance', () => {
  it('lifts very dark light colours so the glyph stays readable', () => {
    expect(glyphColor('#FFFFFF')).toBe('rgb(255,255,255)');
    const [r] = glyphColor('#000000').match(/\d+/g)!.map(Number);
    expect(r).toBeGreaterThan(150);
    expect(glyphColor('#FFB46B')).toBe('rgb(255,180,107)');
  });

  it('names where an edge-pinned light is', () => {
    expect(markerTitle(marker())).toBe('Point light');
    expect(markerTitle(marker({ edge: 0 }))).toBe('Point light — outside the view');
    expect(markerTitle(marker({ edge: 0, behind: true }))).toBe('Point light — behind the camera');
  });

  it('paints every kind of marker without throwing and leaves the context balanced', () => {
    let depth = 0, calls = 0;
    const ctx = new Proxy({} as any, {
      get(target, key) {
        if (key === 'save') return () => { depth++; };
        if (key === 'restore') return () => { depth--; };
        if (key === 'createRadialGradient') return () => ({ addColorStop() {} });
        if (key in target) return target[key];
        return () => { calls++; };
      },
      set(target, key, value) { target[key] = value; return true; },
    });
    const guides = { layerId: 'a', aim: { x: 300, y: 300 }, range: { center: { x: 100, y: 100 }, radius: 60 },
      rim: [{ x: 280, y: 280 }, null, { x: 320, y: 320 }, { x: 330, y: 280 }], innerRim: [{ x: 290, y: 290 }, { x: 310, y: 300 }, { x: 300, y: 310 }] };
    drawLightMarkers(ctx, [
      marker({ selected: true, guides }), marker({ id: 'b', type: 'spot', heading: 1, guides }),
      marker({ id: 'c', type: 'sun', intensity: 0 }), marker({ id: 'd', edge: 2, behind: true, apex: null }),
    ], { accent: '#ff7a33' }, 'b');
    expect(depth).toBe(0);
    expect(calls).toBeGreaterThan(20);
  });
});
