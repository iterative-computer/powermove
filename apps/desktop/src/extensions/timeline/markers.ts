import type { Project, PowermoveAPI } from 'powermove';
type Marker = Project['markers'][number];

/** UI additions land on a composition frame and do not stack pins on that frame. */
export function markerAtTime(api: PowermoveAPI, time: number): Marker | undefined {
  const project = api.project.get();
  return project.markers.find(marker => Math.abs(marker.t - time) < .5 / project.fps);
}

export function addTimelineMarker(api: PowermoveAPI, time = api.transport.time()): void {
  const project = api.project.get();
  if (!Number.isFinite(time)) return;
  time = api.util.clamp(api.util.snapF(time, project.fps), 0, project.dur);
  if (markerAtTime(api, time)) return;
  const result = api.edit.apply({ type: 'add_marker', time }, { label: 'Add marker', origin: 'timeline' });
  if (!result.ok) api.ui.toast(result.message);
}

export function removeTimelineMarker(api: PowermoveAPI, marker: Marker): void {
  // Resolve at invocation time: another edit may have reordered the list while
  // its menu was open. Identity also supports older projects without marker IDs.
  const index = api.project.get().markers.findIndex(item => marker.id ? item.id === marker.id : item === marker);
  if (index < 0) return;
  const result = api.edit.apply({ type: 'remove_marker', index }, { label: 'Remove marker', origin: 'timeline' });
  if (!result.ok) api.ui.toast(result.message);
}

export function timelineMarkerHit(markers: readonly Marker[], x: number, y: number,
  gutter: number, width: number, ruler: number, t2x: (time: number) => number): Marker | undefined {
  if (x < gutter || x > width || y < ruler - 13 || y > ruler) return;
  let hit: Marker | undefined, distance = 8;
  for (const marker of markers) {
    const px = t2x(marker.t);
    const next = Math.abs(px - x);
    if (Number.isFinite(px) && px >= gutter && px <= width && next < distance) { hit = marker; distance = next; }
  }
  return hit;
}

export function drawTimelineMarkers(c: CanvasRenderingContext2D, markers: readonly Marker[],
  gutter: number, width: number, height: number, ruler: number, t2x: (time: number) => number,
  color: string): void {
  c.save();
  c.beginPath(); c.rect(gutter, 0, Math.max(0, width - gutter), height); c.clip();
  c.fillStyle = color; c.strokeStyle = color; c.lineWidth = 1;
  for (const marker of markers) {
    const px = t2x(marker.t);
    if (!Number.isFinite(px) || px < gutter || px > width) continue;
    const x = Math.round(px) + .5;
    c.globalAlpha = .35;
    c.beginPath(); c.moveTo(x, ruler); c.lineTo(x, height); c.stroke();
    c.globalAlpha = 1;
    c.beginPath(); c.moveTo(x - 4, ruler - 11); c.lineTo(x + 4, ruler - 11);
    c.lineTo(x + 4, ruler - 5); c.lineTo(x, ruler - 1); c.lineTo(x - 4, ruler - 5);
    c.closePath(); c.fill();
  }
  c.restore();
}
