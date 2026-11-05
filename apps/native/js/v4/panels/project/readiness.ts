import type { Projection, Row } from "./contract";

/** Presentation-only age. Original readiness remains Unknown in the model, so
 * the Loading treatment cannot enable preview, download or timeline dragging. */
export class UnknownReadiness {
  private first = new Map<string, number>();
  private document: string | null = null;
  observe(view: Projection | null, now: number) {
    const owner = view
      ? `${view.page.stamp.project}:${view.page.stamp.document}`
      : null;
    if (owner !== this.document) {
      this.first.clear();
      this.document = owner;
    }
    for (const row of view?.page.rows ?? []) {
      if (!("Asset" in row.data)) continue;
      const [asset, runtime] = row.data.Asset;
      if (runtime.availability !== "Unknown") this.first.delete(asset.id);
      else if (!this.first.has(asset.id)) this.first.set(asset.id, now);
    }
  }
  loading(row: Row, now: number): boolean {
    if (!("Asset" in row.data) || row.data.Asset[1].availability !== "Unknown")
      return false;
    const first = this.first.get(row.data.Asset[0].id);
    return first !== undefined && now - first >= 2000;
  }
  next(view: Projection, now: number): number | null {
    let delay = Infinity;
    for (const row of view.page.rows) {
      if (
        !("Asset" in row.data) ||
        row.data.Asset[1].availability !== "Unknown"
      )
        continue;
      const first = this.first.get(row.data.Asset[0].id);
      if (first !== undefined && now - first < 2000)
        delay = Math.min(delay, 2000 - (now - first));
    }
    return Number.isFinite(delay) ? delay : null;
  }
}
