import { timelinePanelOptions } from '../legacy/ui/timeline';
import { viewerPanelOptions } from '../legacy/ui/viewer';
import TimelinePanel from './TimelinePanel.svelte';
import ViewerPanel from './ViewerPanel.svelte';
import { registerSveltePanel } from './registerSveltePanel';

type LegacyPM = Record<string, any>;

export function registerCanvasPanels(PM: LegacyPM): void {
  registerSveltePanel(PM, 'viewer', { ...viewerPanelOptions, component: ViewerPanel });
  registerSveltePanel(PM, 'timeline', { ...timelinePanelOptions, component: TimelinePanel });
}
