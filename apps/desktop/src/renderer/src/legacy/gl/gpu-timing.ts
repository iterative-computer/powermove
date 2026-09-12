import { performanceMonitor, type PerformanceIssue } from '../../runtime/performance-monitor';
type Label = Omit<PerformanceIssue, 'ms' | 'samples' | 'at'>;

/** Asynchronous GPU queries: never finish(), read pixels, or wait for a result. */
export class GPUTiming {
  private ext: any;
  private pending: Array<{ query: WebGLQuery; label: Label }> = [];
  private sampled = new Map<string, number>();
  constructor(private gl: WebGL2RenderingContext) {
    this.ext = gl.getExtension('EXT_disjoint_timer_query_webgl2');
  }
  measure(label: Label, draw: () => void): void {
    const now = performance.now();
    if (!this.ext || this.pending.length >= 16 || now - (this.sampled.get(label.id) ?? -Infinity) < 500) { draw(); return; }
    const query = this.gl.createQuery();
    if (!query) { draw(); return; }
    this.sampled.delete(label.id); this.sampled.set(label.id, now);
    while (this.sampled.size > 128) this.sampled.delete(this.sampled.keys().next().value!);
    this.gl.beginQuery(this.ext.TIME_ELAPSED_EXT, query);
    try { draw(); } finally {
      this.gl.endQuery(this.ext.TIME_ELAPSED_EXT);
      this.pending.push({ query, label });
    }
  }
  poll(): void {
    if (!this.ext || !this.pending.length) return;
    const disjoint = this.gl.getParameter(this.ext.GPU_DISJOINT_EXT);
    this.pending = this.pending.filter(({ query, label }) => {
      if (!disjoint && !this.gl.getQueryParameter(query, this.gl.QUERY_RESULT_AVAILABLE)) return true;
      if (!disjoint) performanceMonitor.record(label, this.gl.getQueryParameter(query, this.gl.QUERY_RESULT) / 1e6);
      this.gl.deleteQuery(query);
      return false;
    });
  }
  dispose(): void {
    for (const { query } of this.pending) this.gl.deleteQuery(query);
    this.pending = []; this.sampled.clear();
  }
}
