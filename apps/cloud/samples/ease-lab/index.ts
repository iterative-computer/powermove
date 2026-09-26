import type { PowermoveAPI } from 'powermove';
import Panel from './Panel.svelte';

export default function activate(api: PowermoveAPI) {
  api.commands.register({
    id: `${api.id}.apply`, label: 'Apply Ease Lab preset', category: 'Ease Lab',
    run: async () => {
      await api.project.apply([]);
      api.ui.toast('Applied');
    },
  });
  api.keybindings.bind({ key: 'alt+shift+e', command: `${api.id}.apply` });
  api.panels.register({ id: `${api.id}.panel`, title: 'Ease lab', icon: 'puzzle', size: 280, component: Panel });
}
