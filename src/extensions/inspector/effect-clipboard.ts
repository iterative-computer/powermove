type EffectLike = {
  type: string;
  on?: boolean;
  open?: boolean;
  p?: Record<string, unknown>;
};

export type PasteEffectCommand = {
  type: 'add_effect';
  target: string;
  effect: string;
  parameters: Record<string, unknown>;
  open: boolean;
  enabled: boolean;
};

type EffectSnapshot = Omit<PasteEffectCommand, 'type' | 'target'>;

let clipboard: EffectSnapshot[] = [];

const clone = <T>(value: T): T => {
  if (typeof structuredClone === 'function') return structuredClone(value);
  return JSON.parse(JSON.stringify(value)) as T;
};

export function copyEffects(effects: EffectLike[]): number {
  clipboard = effects.map((effect) => ({
    effect: effect.type,
    parameters: clone(effect.p ?? {}),
    open: effect.open !== false,
    enabled: effect.on !== false,
  }));
  return clipboard.length;
}

export function effectPasteCommands(target: string): PasteEffectCommand[] {
  return clipboard.map((effect) => ({ type: 'add_effect', target, ...clone(effect) }));
}

export function clearEffectClipboard(): void {
  clipboard = [];
}
