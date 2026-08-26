import type { KeybindingDefinition } from 'powermove';

/**
 * Every keybinding implemented by the original editor keydown handler.
 * `cmd+…` rows also have `ctrl+…` equivalents because the old handler tested
 * `metaKey || ctrlKey`.
 */
export const KEYMAP_DEFAULT: KeybindingDefinition[] = [];

const bind = (key: string, command: string, looseModifiers = false): void => {
  KEYMAP_DEFAULT.push({ key, command, priority: 100, ...(looseModifiers ? { looseModifiers: true } : {}) });
};

/* Modifier chords: the old `if (m && …)` chain. `m` was meta OR ctrl. */
const MOD_CHORDS: Array<[string, string, boolean]> = [
  ['k', 'palette', true], ['shift+k', 'agent', true],
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
  ['s', 'save', true], ['shift+s', 'takeSave', true],
  ['o', 'open', true], ['shift+o', 'open', true],
  ['e', 'export', true], ['shift+e', 'export', true],
  ['p', 'projects', false],
  ['n', 'newProject', true], ['shift+n', 'newProject', true],
  /* `if (k === 'F9') go(m ? 'easeLinear' : …)` — meta wins over shift. */
  ['f9', 'easeLinear', true], ['shift+f9', 'easeLinear', true]
];
for (const [chord, command, looseModifiers] of MOD_CHORDS) {
  bind(`cmd+${chord}`, command, looseModifiers);
  bind(`ctrl+${chord}`, command, looseModifiers);
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
