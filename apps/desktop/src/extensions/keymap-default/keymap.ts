import { propertyShortcuts, type KeybindingDefinition } from 'powermove';

/**
 * The built-in editor keymap: legacy AE-style controls plus strict
 * editor-workflow shortcuts. `cmd+…` rows also have `ctrl+…` equivalents so
 * the same map works on macOS and Windows/Linux.
 */
export const KEYMAP_DEFAULT: KeybindingDefinition[] = [];

const REPEATABLE_COMMANDS = new Set(['nextFrame', 'prevFrame', 'stepFrames', 'nudgeSelection', 'nudgeKeyframes']);

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
  ['t', 'toolText', true], ['shift+t', 'toolText', true],
  ['d', 'duplicate', true], ['shift+d', 'split', true],
  ['c', 'copyLayers', true], ['shift+c', 'groupLayers', true], ['g', 'groupLayers', true], ['shift+g', 'ungroupLayers', true],
  /* Paste and Projects explicitly rejected Shift. */
  ['v', 'contextPaste', false],
  ['a', 'selectAll', true], ['shift+a', 'deselect', true],
  ['i', 'import', true], ['shift+i', 'import', true],
  ['s', 'save', false], ['shift+s', 'saveAs', false],
  ['o', 'open', true], ['shift+o', 'open', true],
  ['e', 'export', true], ['shift+e', 'export', true],
  ['p', 'projects', false],
  ['n', 'newProject', false],
];
for (const [chord, command, looseModifiers] of MOD_CHORDS) {
  /* File commands are exact so a shifted chord is never caught by the plain
     command's legacy subset matcher. Cmd/Ctrl+Shift+E remains an explicit
     export alias for now. */
  const exactFileChord = command === 'export' || command === 'newProject';
  const loose = exactFileChord ? false : looseModifiers;
  bind(`cmd+${chord}`, command, loose);
  bind(`ctrl+${chord}`, command, loose);
  if (command === 'save' || command === 'saveAs' || command === 'settings' || command === 'contextPaste') {
    KEYMAP_DEFAULT[KEYMAP_DEFAULT.length - 1]!.inFields = true;
    KEYMAP_DEFAULT[KEYMAP_DEFAULT.length - 2]!.inFields = true;
  }
}

/* Exact primary-modifier shortcuts from the editor-style workflow. Keep
   these strict: an extra modifier should not turn into an accidental alias. */
const PRO_CHORDS: Array<[string, string]> = [
  ['b', 'split'],
  ['x', 'cutLayers'],
  ['shift+h', 'toggleLayerControls'],
  ['l', 'lockSelectedLayers'],
  ['shift+l', 'unlockAllLayers'],
  ['up', 'selectPreviousLayer'],
  ['down', 'selectNextLayer'],
  ['shift+up', 'extendSelectionPreviousLayer'],
  ['shift+down', 'extendSelectionNextLayer'],
  ['=', 'zoomIn'],
  /* KeyboardEvent.key reports Shift+= as `+`, so retain the shift modifier
     while spelling the literal plus key as a trailing `+`. */
  ['shift++', 'zoomIn'],
  ['-', 'zoomOut'],
  ['0', 'fitComposition'],
  ['1', 'actualSize'],
  ['alt+home', 'centerAnchor'],
  [']', 'bringForward'],
  ['[', 'sendBackward'],
  ['shift+]', 'bringToFront'],
  ['shift+[', 'sendToBack']
];
for (const [chord, command] of PRO_CHORDS) {
  bind(`cmd+${chord}`, command);
  bind(`ctrl+${chord}`, command);
}

/* AE nudges layers with the bare arrow keys and moves selected keyframes in
   time with Option/Alt+Left/Right. Both variants repeat while held. */
const NUDGE_KEYS: Array<[string, [number, number]]> = [
  ['left', [-1, 0]],
  ['right', [1, 0]],
  ['up', [0, -1]],
  ['down', [0, 1]]
];
for (const [key, delta] of NUDGE_KEYS) {
  bind(key, 'nudgeSelection', false, delta);
  bind(`shift+${key}`, 'nudgeSelection', false, [delta[0] * 10, delta[1] * 10]);
}
bind('alt+left', 'nudgeKeyframes', false, [-1]);
bind('alt+right', 'nudgeKeyframes', false, [1]);
bind('alt+shift+left', 'nudgeKeyframes', false, [-10]);
bind('alt+shift+right', 'nudgeKeyframes', false, [10]);

/* Composition transport follows AE: Page Up/Down steps frames; Home/End move
   to the composition boundaries. Space remains Powermove's global preview. */
const TRANSPORT_CHORDS: Array<[string, string]> = [
  ['space', 'play'],
  ['home', 'gotoStart'], ['end', 'gotoEnd'],
  ['shift+home', 'gotoWorkIn'], ['shift+end', 'gotoWorkOut'],
  ['pageup', 'prevFrame'], ['pagedown', 'nextFrame'],
  ['backspace', 'delete'], ['delete', 'delete'],
  ['escape', 'deselect']
];
for (const [chord, command] of TRANSPORT_CHORDS) {
  bind(chord, command, chord === 'space');
}
bind('shift+pageup', 'stepFrames', false, [-10]);
bind('shift+pagedown', 'stepFrames', false, [10]);
for (const modifier of ['cmd', 'ctrl']) {
  const previous = modifier === 'ctrl' ? 'timeline.adjacentKeyframe:prev' : 'prevFrame';
  const next = modifier === 'ctrl' ? 'timeline.adjacentKeyframe:next' : 'nextFrame';
  bind(`${modifier}+left`, previous);
  bind(`${modifier}+right`, next);
  bind(`${modifier}+shift+left`, 'stepFrames', false, [-10]);
  bind(`${modifier}+shift+right`, 'stepFrames', false, [10]);
}

/* Timeline navigation and layer timing use AE's native muscle memory. */
bind('j', 'prevVisibleEvent');
bind('k', 'nextVisibleEvent');
bind('shift+j', 'timeline.adjacentKeyframe:prev');
bind('shift+k', 'timeline.adjacentKeyframe:next');
bind('i', 'gotoLayerIn');
bind('o', 'gotoLayerOut');
bind('[', 'moveLayerIn');
bind(']', 'moveLayerOut');
bind('alt+[', 'trimIn');
bind('alt+]', 'trimOut');
bind('alt+home', 'moveLayerInToStart');
bind('alt+end', 'moveLayerOutToEnd');

/* Viewer magnification and Graph Editor keys mirror the AE defaults. */
bind('.', 'zoomIn');
bind(',', 'zoomOut');
bind('/', 'actualSize');
bind('shift+/', 'fitComposition');
bind('shift+f3', 'graph');

/* AE's three standard temporal easing shortcuts. */
bind('f9', 'easyEase');
bind('shift+f9', 'easyEaseIn');
bind('cmd+shift+f9', 'easyEaseOut');
bind('ctrl+shift+f9', 'easyEaseOut');

/* Bare keys ignored alt/meta but preserved Shift. */
const BARE_KEYS: Array<[string, string]> = [
  ['v', 'toolSelect'], ['h', 'toolHand'], ['z', 'toolZoom'],
  ['w', 'toolRotate'], ['y', 'toolAnchor'], ['q', 'toolShape'], ['g','toolPen'],
  ['b', 'workIn'], ['n', 'workOut'],
];
for (const [chord, command] of BARE_KEYS) {
  bind(chord, command);
  bind(`shift+${chord}`, command);
}
const PROPERTY_SHORTCUTS = ['p', 's', 'r', 't', 'a', 'u', 'm', 'f', 'e', 'l'] as const;
for (const key of PROPERTY_SHORTCUTS) {
  const command = `timeline.revealProperty:${key}`;
  bind(key, command);
  bind(`shift+${key}`, command, false, [true]);
}
bind('cmd+`', 'timeline.revealAll');
bind('ctrl+`', 'timeline.revealAll');

/* Escape inside a text field blurs it without triggering the editor-level
   deselect binding. The command itself remains in legacy shortcuts because it
   needs the live DOM and is intentionally hidden from the palette. */
KEYMAP_DEFAULT.push({
  key: 'escape',
  command: 'blurField',
  inFields: true,
  priority: 50
});
