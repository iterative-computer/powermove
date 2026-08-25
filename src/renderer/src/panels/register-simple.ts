import AssetsPanel from './AssetsPanel.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import NotesPanel from './NotesPanel.svelte';
import TakesPanel from './TakesPanel.svelte';
import WorkspacesPanel from './WorkspacesPanel.svelte';
import '../controls/controls.css';
import './panels.css';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function registerSimplePanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'assets', { title: 'Media', size: 200, component: AssetsPanel });
  registerSveltePanel(PM, 'fxbrowser', { title: 'Effects', size: 220, component: FxBrowserPanel });
  registerSveltePanel(PM, 'workspaces', { title: 'Workspaces', size: 200, component: WorkspacesPanel });
  registerSveltePanel(PM, 'takes', { title: 'Takes', size: 180, component: TakesPanel });
  registerSveltePanel(PM, 'notes', { title: 'Notes', size: 180, component: NotesPanel });
}
