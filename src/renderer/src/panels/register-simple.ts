import AssetsPanel from './AssetsPanel.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import NotesPanel from './NotesPanel.svelte';
import TakesPanel from './TakesPanel.svelte';
import WorkspacesPanel from './WorkspacesPanel.svelte';
import '../controls/controls.css';
import './panels.css';
import { registerSveltePanel } from './registerSveltePanel';
import { mount } from 'svelte';
import Segmented from '../controls/Segmented.svelte';
import { FX_KIND_OPTIONS, fxBrowser } from './fx-browser.svelte';

type LegacyPM = Record<string, any>;

/* Panel actions live in the header row, beside the options button — never as
   a floating button inside the content area. */
function headerAction(PM: LegacyPM, hdr: HTMLElement, icon: string, label: string, run: () => void): void {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'iconbtn panel-action';
  button.title = label;
  button.setAttribute('aria-label', label);
  const glyph = PM.icon?.(icon);
  if (glyph instanceof Node) button.appendChild(glyph);
  button.addEventListener('click', run);
  const options = hdr.querySelector('.panel-options');
  if (options) hdr.insertBefore(button, options);
  else hdr.appendChild(button);
}

export function registerSimplePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'assets', {
    title: 'Media',
    size: 200,
    component: AssetsPanel,
    header: (hdr) => headerAction(PM, hdr, 'plus', 'Import media (⌘I)', () => PM.pickFiles())
  });
  registerSveltePanel(PM, 'fxbrowser', {
    title: 'Effects & Transitions', size: 240, component: FxBrowserPanel,
    /* The segmented control is the title: the header names what the list is. */
    header: (hdr) => {
      const slot = document.createElement('span');
      slot.className = 'fxb-kind';
      hdr.querySelector('.ptitle')?.after(slot);
      mount(Segmented, {
        target: slot,
        props: {
          label: 'Browse',
          options: FX_KIND_OPTIONS,
          get value() { return fxBrowser.kind; },
          onChange: (id: string) => { fxBrowser.kind = id as typeof fxBrowser.kind; }
        }
      });
      headerAction(PM, hdr, 'search', 'Search', () => {
        fxBrowser.searchOpen = !fxBrowser.searchOpen;
        if (!fxBrowser.searchOpen) fxBrowser.query = '';
      });
    }
  });
  registerSveltePanel(PM, 'workspaces', { title: 'Workspaces', size: 200, component: WorkspacesPanel });
  registerSveltePanel(PM, 'takes', { title: 'Takes', size: 180, component: TakesPanel });
  registerSveltePanel(PM, 'notes', { title: 'Notes', size: 180, component: NotesPanel });
}
