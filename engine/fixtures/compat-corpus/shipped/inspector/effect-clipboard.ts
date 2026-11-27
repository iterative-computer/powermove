import { isProperty } from 'powermove';
type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
type JsonObject = { [key: string]: JsonValue };
type EffectLike = {
  type: string;
  on?: boolean | { v: boolean; kf: unknown[]; expr?: string | null };
  open?: boolean;
  p?: Record<string, unknown>;
};

export type PasteEffectCommand = {
  type: 'add_effect';
  target: string;
  effect: string;
  parameters: JsonObject;
  open: boolean;
  enabled: boolean;
  enabledAnimation?: JsonObject;
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
    parameters: clone(effect.p ?? {}) as JsonObject,
    open: effect.open !== false,
    enabled: isProperty(effect.on) ? (effect.on as any).v !== false : effect.on !== false,
    ...(isProperty(effect.on) ? { enabledAnimation: clone(effect.on as unknown as JsonObject) } : {}),
  }));
  return clipboard.length;
}

export function effectPasteCommands(target: string): PasteEffectCommand[] {
  return clipboard.map((effect) => ({ type: 'add_effect', target, ...clone(effect) }));
}

export function effectClipboardSize(): number {
  return clipboard.length;
}

export function clearEffectClipboard(): void {
  clipboard = [];
}
