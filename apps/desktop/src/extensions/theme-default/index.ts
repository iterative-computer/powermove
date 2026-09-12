import type { PowermoveAPI, ThemeDefinition } from 'powermove';

export const THEMES: ThemeDefinition[] = [
  {
    id: 'default',
    name: 'Powermove',
    scheme: 'auto',
    tokens: {}
  },
  {
    id: 'high-contrast',
    name: 'High contrast',
    scheme: 'auto',
    tokens: {
      '--bg-window': '#FFFFFF',
      '--bg-panel': '#FFFFFF',
      '--bg-field': '#FFFFFF',
      '--tx': '#000000',
      '--tx-2': '#242424',
      '--line': '#555555',
      '--line-strong': '#202020',
      '--focus-ring': '0 0 0 2px #FFFFFF,0 0 0 5px #005FCC'
    },
    darkTokens: {
      '--bg-window': '#000000',
      '--bg-panel': '#080808',
      '--bg-field': '#101010',
      '--tx': '#FFFFFF',
      '--tx-2': '#E0E0E0',
      '--line': '#A8A8A8',
      '--line-strong': '#E8E8E8',
      '--focus-ring': '0 0 0 2px #000000,0 0 0 5px #66AFFF'
    }
  }
];

export default function activate(api: PowermoveAPI): void {
  for (const theme of THEMES) api.theme.register(theme);
}
