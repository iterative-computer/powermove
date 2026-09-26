import type { PowermoveAPI } from 'powermove';

export default function activate(api: PowermoveAPI) {
  api.commands.register({
    id: `${api.id}.run`, label: 'Check Colour Match connection', category: 'Colour Match',
    run: async () => {
      const key = api.vars.get('OPENAI_API_KEY');
      if (!key) {
        api.ui.toast('Set up your OpenAI API key first.', { error: true });
        return;
      }
      try {
        const response = await fetch('https://api.openai.com/v1/models', {
          headers: { Authorization: `Bearer ${key}` },
        });
        api.ui.toast(`Colour Match connection: HTTP ${response.status}`);
      } catch {
        api.ui.toast('Colour Match could not reach the network.', { error: true });
      }
    },
  });
}
