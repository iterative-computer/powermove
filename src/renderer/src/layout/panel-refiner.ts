import { flushSync, mount, unmount } from 'svelte';
import PanelRefiner from '../panels/agent/PanelRefiner.svelte';
import type { PMRegistry } from '../legacy/registry';

/** Only the active panel gets a composer. Draft, attachments and run state are shared. */
export function installPanelRefiner(PM: PMRegistry): void {
  let current: { id: string; host: HTMLElement; component: ReturnType<typeof mount> } | null = null;
  const close = () => {
    if (!current) return;
    const previous = current; current = null;
    void unmount(previous.component); previous.host.remove();
    PM.panelInst[previous.id]?.el?.classList.remove('is-refining');
    PM.panelInst[previous.id]?.el?.querySelector('.panel-refine')?.setAttribute('aria-expanded', 'false');
  };
  const open = (id: string) => {
    if (current?.id === id) { current.host.querySelector('textarea')?.focus(); return; }
    const element = PM.panelInst[id]?.el as HTMLElement | undefined;
    if (!element) return;
    close();
    PM.AgentUI?.setScope('panel:' + id);
    const host = document.createElement('div'); host.className = 'panel-refiner-host';
    element.appendChild(host);
    element.classList.add('is-refining');
    const component = mount(PanelRefiner, { target: host, props: { PM, panelId: id, close } });
    current = { id, host, component };
    flushSync();
    element.querySelector('.panel-refine')?.setAttribute('aria-expanded', 'true');
    host.querySelector('textarea')?.focus();
  };
  PM.PanelRefiner = { open, close, get activeId() { return current?.id ?? null; }, toggle: (id: string) => current?.id === id ? close() : open(id) };
  PM.bus.on('project', close);
}
