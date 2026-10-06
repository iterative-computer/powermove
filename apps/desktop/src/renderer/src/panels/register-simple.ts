import AssetsPanel from './AssetsPanel.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import MixerPanel from './MixerPanel.svelte';
import NotesPanel from './NotesPanel.svelte';
import TakesPanel from './TakesPanel.svelte';
import WorkspacesPanel from './WorkspacesPanel.svelte';
import '../controls/controls.css';
import './panels.css';
import { registerSveltePanel } from './registerSveltePanel';
import { mount } from 'svelte';
import { fxBrowser } from './fx-browser.svelte';

type LegacyPM = Record<string, any>;

/* Panel actions live in the header row, beside the options button — never as
   a floating button inside the content area. */
function headerAction(PM: LegacyPM, hdr: HTMLElement, icon: string, label: string, run: (this: HTMLElement) => void): void {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'iconbtn panel-action';
  button.title = label;
  button.setAttribute('aria-label', label);
  const glyph = PM.icon?.(icon);
  if (glyph instanceof Node) button.appendChild(glyph);
  button.addEventListener('click', () => run.call(button));
  const options = hdr.querySelector('.panel-options');
  if (options) hdr.insertBefore(button, options);
  else hdr.appendChild(button);
}

export function registerSimplePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'assets', {
    title: 'Media',
    size: 200,
    component: AssetsPanel,
    /* AE's Project panel: new compositions and imported footage live together. */
    header: (hdr) => headerAction(PM, hdr, 'plus', 'New composition or import media', function (this: HTMLElement) {
      PM.menu(this, [
        { label: 'New Composition…', kb: '⌘N', run: () => PM.cmd('newComposition') },
        { label: 'Import Media…', kb: '⌘I', run: () => PM.pickFiles() },
        { label: 'Import Image Sequence…', run: () => PM.pickFiles(true) },
      ]);
    })
  });
  registerSveltePanel(PM, 'fxbrowser', {
    title: 'Effects', size: 240, component: FxBrowserPanel,
    header: (hdr) => headerAction(PM, hdr, 'search', 'Search', () => {
      fxBrowser.searchOpen = !fxBrowser.searchOpen;
      if (!fxBrowser.searchOpen) fxBrowser.query = '';
    })
  });
  /* Channel strips with live meters; docks anywhere (a tall sidebar or a wide strip under the viewer). */
  registerSveltePanel(PM, 'mixer', { title: 'Mixer', size: 300, min: 150, noscroll: true, component: MixerPanel });
  registerSveltePanel(PM, 'workspaces', { title: 'Workspaces', size: 200, component: WorkspacesPanel });
  registerSveltePanel(PM, 'takes', { title: 'Takes', size: 180, component: TakesPanel });
  registerSveltePanel(PM, 'notes', { title: 'Notes', size: 180, component: NotesPanel });
}
