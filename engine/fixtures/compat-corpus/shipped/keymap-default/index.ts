import type { PowermoveAPI } from 'powermove';

import { KEYMAP_DEFAULT } from './keymap';

export default function activate(api: PowermoveAPI): void {
  for (const definition of KEYMAP_DEFAULT) api.keybindings.bind(definition);
}
