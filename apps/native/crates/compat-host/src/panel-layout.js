/* Host-owned geometry bypasses the extension worker's DOM round trip. */
globalThis.PMPanelLayout = class {
  constructor(frame, apply, acknowledge) {
    this.frame = frame;
    this.apply = apply;
    this.acknowledge = acknowledge;
    this.latest = new Map();
    this.pending = new Map();
    this.placements = new Map();
    this.scheduled = false;
  }
  enqueue(placements) {
    for (const placement of placements) {
      if (placement.generation <= (this.latest.get(placement.id) || 0)) continue;
      this.latest.set(placement.id, placement.generation);
      this.pending.set(placement.id, placement);
      this.placements.set(placement.id, placement);
    }
    this.schedule();
  }
  refresh() {
    for (const [id, placement] of this.placements) this.pending.set(id, placement);
    this.schedule();
  }
  schedule() {
    if (this.scheduled || !this.pending.size) return;
    this.scheduled = true;
    this.frame(() => {
      this.scheduled = false;
      const applied = [...this.pending.values()];
      this.pending.clear();
      const rendered = applied.filter((placement) => this.apply(placement) !== false);
      // The second animation frame follows the layout frame's paint opportunity.
      this.frame(() => {
        for (const placement of rendered)
          if (this.latest.get(placement.id) === placement.generation)
            this.acknowledge(placement);
      });
    });
  }
};
