/*
 * Key chord parsing/normalisation — the single definition of the shortcut
 * syntax documented on `KeybindingsAPI` in api.ts.
 *
 * Canonical form: `cmd+ctrl+alt+shift+<key>`, all lowercase, modifiers always in
 * that order, key names spelled out for anything longer than one character
 * (`space`, `escape`, `up`, `f9`, …). Two chords are equal iff their canonical
 * strings are equal, so the keybinding registry can key straight off the string.
 */

const MOD_ALIASES: Record<string, 'cmd' | 'ctrl' | 'alt' | 'shift'> = {
  cmd: 'cmd',
  command: 'cmd',
  meta: 'cmd',
  super: 'cmd',
  win: 'cmd',
  '⌘': 'cmd',
  ctrl: 'ctrl',
  control: 'ctrl',
  '^': 'ctrl',
  '⌃': 'ctrl',
  alt: 'alt',
  option: 'alt',
  opt: 'alt',
  '⌥': 'alt',
  shift: 'shift',
  '⇧': 'shift'
};

const KEY_ALIASES: Record<string, string> = {
  ' ': 'space',
  spacebar: 'space',
  space: 'space',
  esc: 'escape',
  escape: 'escape',
  return: 'enter',
  enter: 'enter',
  del: 'delete',
  arrowup: 'up',
  arrowdown: 'down',
  arrowleft: 'left',
  arrowright: 'right',
  pgup: 'pageup',
  pgdn: 'pagedown',
  plus: '+',
  minus: '-',
  equal: '=',
  equals: '='
};

/** Keys that never form a chord on their own. */
const BARE_MODIFIERS = new Set(['shift', 'control', 'alt', 'meta', 'altgraph', 'capslock', 'numlock', 'scrolllock', 'os', 'dead', 'unidentified', 'fn', 'fnlock']);

function normalizeKeyName(raw: string): string {
  const key = raw.toLowerCase();
  return KEY_ALIASES[key] ?? key;
}

/**
 * Canonicalise a chord string. Unknown modifier-looking tokens are treated as
 * part of the key name so a typo surfaces as a chord that simply never matches
 * rather than as a silently dropped modifier.
 */
export function normalizeChord(input: string): string {
  const raw = String(input ?? '');
  // A literal space IS a chord (`space`), so it survives the whitespace guard.
  if (!raw.trim() && raw !== ' ') return '';

  const tokens = raw.split('+').map((token) => (token === ' ' ? 'space' : token.trim()));
  // A trailing empty token means the key itself is `+` (e.g. `cmd++`, `+`).
  let literalPlus = false;
  while (tokens.length > 1 && tokens[tokens.length - 1] === '') {
    tokens.pop();
    literalPlus = true;
  }
  if (tokens.length === 1 && tokens[0] === '') literalPlus = true;

  const mods = new Set<'cmd' | 'ctrl' | 'alt' | 'shift'>();
  let key = '';
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i] ?? '';
    if (!token) continue;
    const mod = MOD_ALIASES[token.toLowerCase()];
    if (mod && i < tokens.length - 1) {
      mods.add(mod);
      continue;
    }
    if (mod && i === tokens.length - 1 && literalPlus) {
      mods.add(mod);
      continue;
    }
    key = normalizeKeyName(token);
  }
  if (literalPlus) key = '+';
  if (!key) return '';

  const order: Array<'cmd' | 'ctrl' | 'alt' | 'shift'> = ['cmd', 'ctrl', 'alt', 'shift'];
  return [...order.filter((m) => mods.has(m)), key].join('+');
}

/**
 * Normalise a KeyboardEvent to chord syntax, or null when only modifiers are
 * held. Letters/digits/punctuation come from `event.key`; when shift turns a
 * digit row key into a symbol we fall back to the physical `event.code` so
 * `shift+1` stays `shift+1` instead of becoming `shift+!`.
 */
export function chordOfEvent(event: KeyboardEvent): string | null {
  const raw = event?.key;
  if (typeof raw !== 'string' || raw.length === 0) return null;
  if (BARE_MODIFIERS.has(raw.toLowerCase())) return null;

  let key = raw.length > 1 ? normalizeKeyName(raw) : raw.toLowerCase();
  const code = typeof event.code === 'string' ? event.code : '';
  if (event.shiftKey && /^Digit\d$/.test(code) && !/^[a-z0-9]$/.test(key)) key = code.slice(5);
  if (raw === ' ') key = 'space';

  const parts: string[] = [];
  if (event.metaKey) parts.push('cmd');
  if (event.ctrlKey) parts.push('ctrl');
  if (event.altKey) parts.push('alt');
  if (event.shiftKey) parts.push('shift');
  parts.push(key);
  return normalizeChord(parts.join('+'));
}

interface ChordParts {
  key: string;
  modifiers: Set<'primary' | 'alt' | 'shift'>;
}

function chordParts(chord: string): ChordParts | null {
  let rest = normalizeChord(chord);
  if (!rest) return null;

  const modifiers = new Set<'primary' | 'alt' | 'shift'>();
  for (const modifier of ['cmd', 'ctrl', 'alt', 'shift'] as const) {
    const prefix = `${modifier}+`;
    if (!rest.startsWith(prefix)) continue;
    rest = rest.slice(prefix.length);
    modifiers.add(modifier === 'cmd' || modifier === 'ctrl' ? 'primary' : modifier);
  }
  return rest ? { key: rest, modifiers } : null;
}

/** Match a pressed chord against a binding, optionally using legacy modifier semantics. */
export function chordMatches(bindingChord: string, pressedChord: string, looseModifiers = false): boolean {
  const binding = normalizeChord(bindingChord);
  const pressed = normalizeChord(pressedChord);
  if (!binding || !pressed) return false;
  if (binding === pressed) return true;
  if (!looseModifiers) return false;

  const wanted = chordParts(binding);
  const actual = chordParts(pressed);
  if (!wanted || !actual || wanted.key !== actual.key) return false;
  for (const modifier of wanted.modifiers) {
    if (!actual.modifiers.has(modifier)) return false;
  }
  return true;
}

/** Number of semantic modifiers in a chord; cmd and ctrl are one equivalent modifier. */
export function chordModifierCount(chord: string): number {
  return chordParts(chord)?.modifiers.size ?? 0;
}

/** True when the event target is a text-entry surface that should swallow keys. */
export function isFieldTarget(target: unknown): boolean {
  const el = target as (HTMLElement & { tagName?: string; isContentEditable?: boolean }) | null;
  if (!el || typeof el !== 'object') return false;
  const tag = typeof el.tagName === 'string' ? el.tagName.toUpperCase() : '';
  if (tag === 'INPUT' || tag === 'TEXTAREA') return true;
  return el.isContentEditable === true;
}
