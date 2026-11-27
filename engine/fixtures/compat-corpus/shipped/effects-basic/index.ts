import type { PowermoveAPI } from 'powermove';

import { EFFECTS } from './effects';

export default function activate(api: PowermoveAPI): void {
  for (const definition of EFFECTS) api.effects.register(definition);
}
