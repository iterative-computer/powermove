// @vitest-environment happy-dom
import { flushSync, mount, unmount, tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import VisualReply from './VisualReply.svelte';
import { prepareVisualReply } from './visual-reply';
const PM={ICONS:{aspectRatio:'<path d="M0 0"/>',plus:'<path d="M0 0"/>',rotate:'<path d="M0 0"/>',x:'<path d="M0 0"/>' }};
describe('visual reply native chat controls', () => {
  beforeEach(() => {
    installBridgeForTests({versions:{electron:'test',chrome:'test',node:'test'}} as never);
    // happy-dom cannot execute opaque browser frames. Exercise host controls
    // here; real frame execution belongs to the Electron security walkthrough.
    (window as any).happyDOM.settings.disableIframePageLoading = true;

  });
  afterEach(() => {resetBridgeForTests();vi.restoreAllMocks();});
  it('mounts an opaque frame and exposes replay, zoom and expansion', async () => {
    const target=document.createElement('div');document.body.append(target);
    const artifact={version:1 as const,id:'visual-controls',runId:'run',...prepareVisualReply({title:'Timing',html:'<h1>Timing</h1>'})};
    const component=mount(VisualReply,{target,props:{PM,artifact}});flushSync();
    try {
      const frame=target.querySelector('iframe')!;
      expect(target.querySelector('.agent-attachment-type')?.textContent).toBe('HTML');
      expect(target.querySelector('[data-icon="aspectRatio"]')).not.toBeNull();
      expect(frame.getAttribute('sandbox')).toBe('allow-scripts');
      expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
      expect(frame.getAttribute('src')).toBe('/visual-reply.html');
      expect(frame.getAttribute('allow')).toContain("clipboard-read 'none'");
      expect(target.querySelector('[aria-label="Zoom in Timing"]')).toBeNull();
      const dialog=target.querySelector('dialog')!;
      dialog.showModal=vi.fn(()=>dialog.setAttribute('open',''));
      dialog.close=vi.fn(()=>{dialog.removeAttribute('open');dialog.dispatchEvent(new Event('close'));});
      target.querySelector<HTMLButtonElement>('[aria-label="Expand Timing"]')!.click();await tick();flushSync();
      expect(dialog.showModal).toHaveBeenCalled();
      expect(dialog.classList.contains('modal')).toBe(true);
      expect(dialog.getAttribute('aria-label')).toBe('Expanded Timing');
      expect(target.querySelector('[data-icon="x"]')).not.toBeNull();
      expect(target.querySelector('[data-icon="rotate"]')).not.toBeNull();
      target.querySelector<HTMLButtonElement>('[aria-label="Zoom in Timing"]')!.click();flushSync();
      expect(target.querySelector('[aria-label="Reset zoom Timing"]')?.textContent).toBe('125%');
      target.querySelector<HTMLButtonElement>('[aria-label="Replay Timing"]')!.click();flushSync();
      expect(target.querySelector('iframe')).not.toBe(frame);

      expect(target.querySelector('iframe')?.getAttribute('sandbox')).toBe('allow-scripts');
      target.querySelector<HTMLButtonElement>('[aria-label="Close visualization"]')!.click();flushSync();
      expect(dialog.close).toHaveBeenCalled();
      expect(target.querySelector('[aria-label="Zoom in Timing"]')).toBeNull();
      expect(target.querySelector('[aria-expanded="false"]')).not.toBeNull();
    } finally {await unmount(component);target.remove();}
  });
  it('keeps browser hosts from running visual scripts without native navigation enforcement', async () => {
    resetBridgeForTests();
    const target=document.createElement('div');document.body.append(target);
    const component=mount(VisualReply,{target,props:{PM,artifact:{version:1,id:'visual-web',runId:'run',title:'Chart',html:'<h1>Chart</h1>',height:300}}});flushSync();
    try {expect(target.querySelector('[role="alert"]')?.textContent).toContain('desktop app');expect(target.querySelector('iframe')).toBeNull();}
    finally {await unmount(component);target.remove();}
  });
  it('reports invalid saved artifacts without loading their code', async () => {
    const target=document.createElement('div');document.body.append(target);
    const component=mount(VisualReply,{target,props:{PM,artifact:{version:1,id:'visual-invalid',runId:'run',title:'Broken',html:'<iframe></iframe>',height:300}}});flushSync();
    try {expect(target.querySelector('[role="alert"]')?.textContent).toContain('cannot embed');expect(target.querySelector('iframe')).toBeNull();}
    finally {await unmount(component);target.remove();}
  });
});
