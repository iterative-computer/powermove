import type { KeybindingDefinition } from 'powermove';

/**
 * The built-in editor keymap: legacy AE-style controls plus strict
 * editor-workflow shortcuts. `cmd+…` rows also have `ctrl+…` equivalents so
 * the same map works on macOS and Windows/Linux.
 */
export const KEYMAP_DEFAULT: KeybindingDefinition[] = [];

const REPEATABLE_COMMANDS = new Set(['nextFrame', 'prevFrame', 'nudgeSelection']);

const bind = (key: string, command: string, looseModifiers = false, args?: unknown[]): void => {
  KEYMAP_DEFAULT.push({
    key,
    command,
    ...(args ? { args } : {}),
    priority: 100,
    ...(REPEATABLE_COMMANDS.has(command) ? { repeat: true } : {}),
    ...(looseModifiers ? { looseModifiers: true } : {})
  });
};

/* Modifier chords: the old `if (m && …)` chain. `m` was meta OR ctrl. */
const MOD_CHORDS: Array<[string, string, boolean]> = [
  ['k', 'palette', true], ['shift+k', 'agent', true],
  [',', 'settings', false],
  ['z', 'undo', true], ['shift+z', 'redo', true],
  /* The plain Y branch explicitly rejected both Shift and Alt. */
  ['y', 'newSolid', false], ['shift+y', 'newShape', true],
  ['t', 'newText', true], ['shift+t', 'newText', true],
  ['shift+g', 'newShader', true],
  ['d', 'duplicate', true], ['shift+d', 'split', true],
  ['c', 'copyLayers', true], ['shift+c', 'precompose', true],
  /* Paste and Projects explicitly rejected Shift. */
  ['v', 'pasteLayers', false],
  ['a', 'selectAll', true], ['shift+a', 'selectAll', true],
  ['i', 'import', true], ['shift+i', 'import', true],
  ['s', 'save', false], ['shift+s', 'saveAs', false],
  ['o', 'open', true], ['shift+o', 'open', true],
  ['e', 'export', true], ['shift+e', 'export', true],
  ['p', 'projects', false],
  ['n', 'newProject', false],
  /* `if (k === 'F9') go(m ? 'easeLinear' : …)` — meta wins over shift. */
  ['f9', 'easeLinear', true], ['shift+f9', 'easeLinear', true]
];
for (const [chord, command, looseModifiers] of MOD_CHORDS) {
  /* File commands are exact so a shifted chord is never caught by the plain
     command's legacy subset matcher. Cmd/Ctrl+Shift+E remains an explicit
     export alias for now. */
  const exactFileChord = command === 'export' || command === 'newProject';
  const loose = exactFileChord ? false : looseModifiers;
  bind(`cmd+${chord}`, command, loose);
  bind(`ctrl+${chord}`, command, loose);
  if (command === 'save' || command === 'saveAs' || command === 'settings') {
    KEYMAP_DEFAULT[KEYMAP_DEFAULT.length - 1]!.inFields = true;
    KEYMAP_DEFAULT[KEYMAP_DEFAULT.length - 2]!.inFields = true;
  }
}

/* Exact primary-modifier shortcuts from the editor-style workflow. Keep
   these strict: an extra modifier should not turn into an accidental alias. */
const PRO_CHORDS: Array<[string, string]> = [
  ['b', 'split'],
  ['x', 'cutLayers'],
  ['shift+h', 'toggleVisibility'],
  ['=', 'zoomIn'],
  /* KeyboardEvent.key reports Shift+= as `+`, so retain the shift modifier
     while spelling the literal plus key as a trailing `+`. */
  ['shift++', 'zoomIn'],
  ['-', 'zoomOut'],
  ['0', 'fitComposition'],
  ['1', 'actualSize'],
  [']', 'bringForward'],
  ['[', 'sendBackward'],
  ['shift+]', 'bringToFront'],
  ['shift+[', 'sendToBack']
];
for (const [chord, command] of PRO_CHORDS) {
  bind(`cmd+${chord}`, command);
  bind(`ctrl+${chord}`, command);
}

/* Alt-drag-style keyboard nudging. Register these before the legacy loose
   transport aliases so the exact nudge chord wins when both describe the same
   physical key. Plain and Shift arrows below retain timeline semantics. */
const NUDGE_KEYS: Array<[string, [number, number]]> = [
  ['left', [-1, 0]],
  ['right', [1, 0]],
  ['up', [0, -1]],
  ['down', [0, 1]]
];
for (const [key, delta] of NUDGE_KEYS) {
  bind(`alt+${key}`, 'nudgeSelection', false, delta);
  bind(`alt+shift+${key}`, 'nudgeSelection', false, [delta[0] * 10, delta[1] * 10]);
}

/* The old `switch (k)` ran before its alt/meta guard, so these work bare or
   with any one of the modifiers represented below. */
const TRANSPORT_CHORDS: Array<[string, string]> = [
  ['space', 'play'],
  ['home', 'gotoStart'], ['end', 'gotoEnd'],
  ['right', 'nextFrame'], ['shift+right', 'nextEdge'],
  ['left', 'prevFrame'], ['shift+left', 'prevEdge'],
  ['backspace', 'delete'], ['delete', 'delete'],
  ['escape', 'deselect']
];
for (const [chord, command] of TRANSPORT_CHORDS) {
  bind(chord, command, true);
  bind(`cmd+${chord}`, command, true);
  bind(`ctrl+${chord}`, command, true);
  bind(`alt+${chord}`, command, true);
}
bind('f9', 'easeOut', true);
bind('shift+f9', 'easePower', true);

/* Bare keys ignored alt/meta but preserved Shift. */
const BARE_KEYS: Array<[string, string]> = [
  ['v', 'toolSelect'], ['h', 'toolHand'], ['z', 'toolZoom'],
  ['p', 'revealPos'], ['s', 'revealScale'], ['r', 'revealRot'],
  ['t', 'revealOpacity'], ['a', 'revealAnchor'], ['u', 'revealKeys'],
  ['g', 'graph'], ['b', 'workIn'], ['n', 'workOut'],
  ['j', 'prevEdge'], ['k', 'transportPause'], ['l', 'transportPlay'],
  ['i', 'trimIn'], ['o', 'trimOut']
];
for (const [chord, command] of BARE_KEYS) {
  bind(chord, command);
  bind(`shift+${chord}`, command);
}
/* `case 'f': if (s) …` — plain F did nothing. */
bind('shift+f', 'fitView');

/* Escape inside a text field blurs it without triggering the editor-level
   deselect binding. The command itself remains in legacy shortcuts because it
   needs the live DOM and is intentionally hidden from the palette. */
KEYMAP_DEFAULT.push({
  key: 'escape',
  command: 'blurField',
  inFields: true,
  priority: 50
});
