import AssetsPanel from './AssetsPanel.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import NotesPanel from './NotesPanel.svelte';
import TakesPanel from './TakesPanel.svelte';
import WorkspacesPanel from './WorkspacesPanel.svelte';
import '../controls/controls.css';
import './panels.css';
import { registerSveltePanel } from './registerSveltePanel';

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
  registerSveltePanel(PM, 'fxbrowser', { title: 'Effects', size: 220, component: FxBrowserPanel });
  registerSveltePanel(PM, 'workspaces', { title: 'Workspaces', size: 200, component: WorkspacesPanel });
  registerSveltePanel(PM, 'takes', { title: 'Takes', size: 180, component: TakesPanel });
  registerSveltePanel(PM, 'notes', { title: 'Notes', size: 180, component: NotesPanel });
}
