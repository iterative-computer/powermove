import type { PowermoveAPI } from 'powermove';

import { BUILTIN_TRANSITIONS } from './transitions';

export default function activate(api: PowermoveAPI): void {
  for (const definition of BUILTIN_TRANSITIONS) api.transitions.register(definition);
}
