/* Host-owned geometry bypasses the extension worker's DOM round trip. */
globalThis.PMPanelLayout = class {
  constructor(frame, apply, acknowledge, visibility = () => {}) {
    this.frame = frame;
    this.visibility = visibility;
    this.active = new Set();
    this.explicitActive = false;
    this.apply = apply;
    this.acknowledge = acknowledge;
    this.latest = new Map();
    this.pending = new Map();
    this.placements = new Map();
    this.scheduled = false;
  }
  enqueue(placements, active) {
    if (active) {
      this.explicitActive = true;
      this.active = new Set(active);
      for (const id of this.placements.keys()) {
        if (!this.active.has(id)) {
          this.placements.delete(id);
          this.pending.delete(id);
        }
      }
    } else {
      for (const placement of placements) this.active.add(placement.id);
    }
    for (const placement of placements) {
      if (placement.generation <= (this.latest.get(placement.id) || 0)) continue;
      this.latest.set(placement.id, placement.generation);
      this.pending.set(placement.id, placement);
      this.placements.set(placement.id, placement);
    }
    if (this.explicitActive) this.visibility(this.active);
    this.schedule();
  }
  refresh() {
    if (this.explicitActive) this.visibility(this.active);
    // The realm's batch can replace a root's inline style after the host has
    // acknowledged it. Repair the same DOM batch before WebKit can paint it.
    // New geometry belongs to the next frame, not this worker batch. Retry
    // settled placements here in case a root mounted late or was replaced.
    const rendered = this.applyPlacements(
      [...this.placements.values()].filter(({ id }) => !this.pending.has(id)),
    );
    if (rendered.length) this.frame(() => this.frame(() => this.confirm(rendered)));
  }
  applyPlacements(placements) {
    return [...placements].filter((placement) => this.apply(placement) !== false);
  }
  confirm(rendered) {
    for (const placement of rendered) {
      if (this.active.has(placement.id) && this.latest.get(placement.id) === placement.generation)
        this.acknowledge(placement);
    }
  }
  schedule() {
    if (this.scheduled || !this.pending.size) return;
    this.scheduled = true;
    this.frame(() => {
      this.scheduled = false;
      const rendered = this.applyPlacements(this.pending.values());
      this.pending.clear();
      if (rendered.length) this.frame(() => this.confirm(rendered));
    });
  }
};
