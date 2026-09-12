import { flushSync, mount, unmount } from 'svelte';

import Palette from './Palette.svelte';
import { inertOverlayHosts, restoreOverlayHosts, type InertSnapshot } from './inert';
import type { OverlayPM } from './types';

export { paletteEntries, scorePaletteMatch, type PaletteEntry } from './palette-model';

type PaletteInstance = ReturnType<typeof mount> & { element(): HTMLElement };

export class PaletteController {
  private instance: PaletteInstance | null = null;
  private element: HTMLElement | null = null;
  private trigger: HTMLElement | null = null;
  private inertSnapshot: InertSnapshot = [];
  private scrimWasOn = false;

  constructor(private readonly PM: OverlayPM) {}

  open(): void {
    if (this.instance) return;
    this.PM.closeMenus?.();
    const scrim = document.getElementById('scrim');
    if (!scrim) throw new Error('PM.palette requires #scrim');
    this.trigger = document.activeElement instanceof HTMLElement && document.activeElement !== document.body
      ? document.activeElement
      : null;
    this.inertSnapshot = inertOverlayHosts();
    this.scrimWasOn = scrim.classList.contains('on');
    scrim.classList.add('on');
    const instance = mount(Palette, {
      target: document.body,
      props: { PM: this.PM, onclose: () => this.close() }
    }) as PaletteInstance;
    this.instance = instance;
    flushSync();
    this.element = instance.element();
    scrim.onclick = () => this.close();
  }

  close(): void {
    const instance = this.instance;
    if (!instance) return;
    this.instance = null;
    const element = this.element;
    this.element = null;
    void unmount(instance);
    element?.remove();
    restoreOverlayHosts(this.inertSnapshot);
    this.inertSnapshot = [];
    const scrim = document.getElementById('scrim');
    if (scrim) {
      scrim.classList.toggle('on', this.scrimWasOn);
      scrim.onclick = null;
    }
    const trigger = this.trigger;
    this.trigger = null;
    if (trigger?.isConnected) trigger.focus({ preventScroll: true });
  }
}
