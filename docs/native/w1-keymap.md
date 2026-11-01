# W1 — shipped keyboard map

Baseline: `v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`). **195 registered binding entries: 194 from keymap-default plus 1 from viewer.** Cmd and Ctrl aliases are separate rows. Native menu equivalents and focus-local keys are separate tables below and are not added to that registry count. Runtime user-extension bindings are unbounded; the registration/override/decline contract is tested in W010 and W040.

Extraction: expand every loop/push in the frozen `keymap.ts`, then include the only other shipped production `api.keybindings.bind` call (`viewer/index.ts:50`). A repository-wide registration search excludes tests and fixture contributions. Command display hints in `shortcuts.ts` are not bindings. No app build or execution is implied.

Acceptance for **each row**: establish its workflow preconditions, press the chord and assert the command with the exact arguments below and the resulting workflow state. Repeat with a held key, an extra modifier, focused input/contenteditable, selected read-only text, a focused button, and a prior extension binding that returns false. Check no double execution through menus. These are test cases, not checked-off native passes.

Ordering: smaller priority runs first, then exact match, then modifier specificity for loose bindings, then registration order. Only literal false declines. Fields skip entries without inFields. Only listed repeat rows execute autorepeat. A throw reports failure and stops. Unknown commands decline. Ordinary non-field keydown currently prevents default before a decline: native must preserve command ordering without porting browser default-prevention quirks.

Chord normalization: modifiers canonicalize as cmd+ctrl+alt+shift+key; aliases include Command/Meta/⌘, Control/⌃, Option/⌥ and Shift/⇧. Space, Escape, Enter, Delete, PageUp/PageDown and arrows normalize; bare modifiers/dead/unidentified keys do not bind. Shifted digits/brackets/slash use physical code fallback. Literal plus is a trailing +, so cmd+shift++ is valid. Loose matching treats Cmd/Ctrl as primary and accepts added modifiers; exact entries do not. See `v1.2.0:apps/desktop/src/renderer/src/kernel/keychord.ts` and `v1.2.0:apps/desktop/src/renderer/src/kernel/registries.ts`.

## Registered entries

| ID | Exact chord | Command | Args | Priority | Fields | Repeat | Loose | Acceptance workflow | v1.2.0 registration |
|---|---|---|---|---:|---|---|---|---|---|
| K001 | cmd+k | palette | [] | 100 | no | no | yes | W017 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L25` |
| K002 | ctrl+k | palette | [] | 100 | no | no | yes | W017 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L25` |
| K003 | cmd+shift+k | agent | [] | 100 | no | no | yes | W025 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L25` |
| K004 | ctrl+shift+k | agent | [] | 100 | no | no | yes | W025 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L25` |
| K005 | cmd+, | settings | [] | 100 | yes | no | no | W012 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L26` |
| K006 | ctrl+, | settings | [] | 100 | yes | no | no | W012 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L26` |
| K007 | cmd+z | undo | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L27` |
| K008 | ctrl+z | undo | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L27` |
| K009 | cmd+shift+z | redo | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L27` |
| K010 | ctrl+shift+z | redo | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L27` |
| K011 | cmd+y | newSolid | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L29` |
| K012 | ctrl+y | newSolid | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L29` |
| K013 | cmd+shift+y | newShape | [] | 100 | no | no | yes | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L29` |
| K014 | ctrl+shift+y | newShape | [] | 100 | no | no | yes | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L29` |
| K015 | cmd+t | toolText | [] | 100 | no | no | yes | W049 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L30` |
| K016 | ctrl+t | toolText | [] | 100 | no | no | yes | W049 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L30` |
| K017 | cmd+shift+t | toolText | [] | 100 | no | no | yes | W049 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L30` |
| K018 | ctrl+shift+t | toolText | [] | 100 | no | no | yes | W049 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L30` |
| K019 | cmd+d | duplicate | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K020 | ctrl+d | duplicate | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K021 | cmd+shift+d | split | [] | 100 | no | no | yes | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K022 | ctrl+shift+d | split | [] | 100 | no | no | yes | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K023 | cmd+c | copyLayers | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K024 | ctrl+c | copyLayers | [] | 100 | no | no | yes | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K025 | cmd+shift+c | precompose | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K026 | ctrl+shift+c | precompose | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K027 | cmd+g | groupLayers | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K028 | ctrl+g | groupLayers | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K029 | cmd+shift+g | ungroupLayers | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K030 | ctrl+shift+g | ungroupLayers | [] | 100 | no | no | yes | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L32` |
| K031 | cmd+v | contextPaste | [] | 100 | yes | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L34` |
| K032 | ctrl+v | contextPaste | [] | 100 | yes | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L34` |
| K033 | cmd+a | selectAll | [] | 100 | no | no | yes | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L35` |
| K034 | ctrl+a | selectAll | [] | 100 | no | no | yes | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L35` |
| K035 | cmd+shift+a | deselect | [] | 100 | no | no | yes | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L35` |
| K036 | ctrl+shift+a | deselect | [] | 100 | no | no | yes | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L35` |
| K037 | cmd+i | import | [] | 100 | no | no | yes | W019 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L36` |
| K038 | ctrl+i | import | [] | 100 | no | no | yes | W019 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L36` |
| K039 | cmd+shift+i | import | [] | 100 | no | no | yes | W019 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L36` |
| K040 | ctrl+shift+i | import | [] | 100 | no | no | yes | W019 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L36` |
| K041 | cmd+s | save | [] | 100 | yes | no | no | W007 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L37` |
| K042 | ctrl+s | save | [] | 100 | yes | no | no | W007 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L37` |
| K043 | cmd+shift+s | saveAs | [] | 100 | yes | no | no | W007 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L37` |
| K044 | ctrl+shift+s | saveAs | [] | 100 | yes | no | no | W007 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L37` |
| K045 | cmd+o | open | [] | 100 | no | no | yes | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L38` |
| K046 | ctrl+o | open | [] | 100 | no | no | yes | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L38` |
| K047 | cmd+shift+o | open | [] | 100 | no | no | yes | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L38` |
| K048 | ctrl+shift+o | open | [] | 100 | no | no | yes | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L38` |
| K049 | cmd+e | export | [] | 100 | no | no | no | W069 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L39` |
| K050 | ctrl+e | export | [] | 100 | no | no | no | W069 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L39` |
| K051 | cmd+shift+e | export | [] | 100 | no | no | no | W069 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L39` |
| K052 | ctrl+shift+e | export | [] | 100 | no | no | no | W069 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L39` |
| K053 | cmd+p | projects | [] | 100 | no | no | no | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L40` |
| K054 | ctrl+p | projects | [] | 100 | no | no | no | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L40` |
| K055 | cmd+n | newComposition | [] | 100 | no | no | no | W009 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L42` |
| K056 | ctrl+n | newComposition | [] | 100 | no | no | no | W009 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L42` |
| K057 | cmd+alt+n | newProject | [] | 100 | no | no | no | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L43` |
| K058 | ctrl+alt+n | newProject | [] | 100 | no | no | no | W006 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L43` |
| K059 | cmd+alt+y | newAdjustment | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L44` |
| K060 | ctrl+alt+y | newAdjustment | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L44` |
| K061 | cmd+alt+k | compositionSettings | [] | 100 | no | no | no | W009 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L45` |
| K062 | ctrl+alt+k | compositionSettings | [] | 100 | no | no | no | W009 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L45` |
| K063 | cmd+b | split | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K064 | ctrl+b | split | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L31` |
| K065 | cmd+x | cutLayers | [] | 100 | no | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L65` |
| K066 | ctrl+x | cutLayers | [] | 100 | no | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L65` |
| K067 | cmd+shift+h | toggleLayerControls | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L66` |
| K068 | ctrl+shift+h | toggleLayerControls | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L66` |
| K069 | cmd+l | lockSelectedLayers | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L67` |
| K070 | ctrl+l | lockSelectedLayers | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L67` |
| K071 | cmd+shift+l | unlockAllLayers | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L68` |
| K072 | ctrl+shift+l | unlockAllLayers | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L68` |
| K073 | cmd+up | selectPreviousLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L69` |
| K074 | ctrl+up | selectPreviousLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L69` |
| K075 | cmd+down | selectNextLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L70` |
| K076 | ctrl+down | selectNextLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L70` |
| K077 | cmd+shift+up | extendSelectionPreviousLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L71` |
| K078 | ctrl+shift+up | extendSelectionPreviousLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L71` |
| K079 | cmd+shift+down | extendSelectionNextLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L72` |
| K080 | ctrl+shift+down | extendSelectionNextLayer | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L72` |
| K081 | cmd+= | zoomIn | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L73` |
| K082 | ctrl+= | zoomIn | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L73` |
| K083 | cmd+shift++ | zoomIn | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L73` |
| K084 | ctrl+shift++ | zoomIn | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L73` |
| K085 | cmd+- | zoomOut | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L77` |
| K086 | ctrl+- | zoomOut | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L77` |
| K087 | cmd+0 | fitComposition | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L78` |
| K088 | ctrl+0 | fitComposition | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L78` |
| K089 | cmd+1 | actualSize | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L79` |
| K090 | ctrl+1 | actualSize | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L79` |
| K091 | cmd+alt+home | centerAnchor | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L80` |
| K092 | ctrl+alt+home | centerAnchor | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L80` |
| K093 | cmd+] | bringForward | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L81` |
| K094 | ctrl+] | bringForward | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L81` |
| K095 | cmd+[ | sendBackward | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L82` |
| K096 | ctrl+[ | sendBackward | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L82` |
| K097 | cmd+shift+] | bringToFront | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L83` |
| K098 | ctrl+shift+] | bringToFront | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L83` |
| K099 | cmd+shift+[ | sendToBack | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L84` |
| K100 | ctrl+shift+[ | sendToBack | [] | 100 | no | no | no | W062 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L84` |
| K101 | left | nudgeSelection | [-1,0] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K102 | shift+left | nudgeSelection | [-10,0] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K103 | right | nudgeSelection | [1,0] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K104 | shift+right | nudgeSelection | [10,0] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K105 | up | nudgeSelection | [0,-1] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K106 | shift+up | nudgeSelection | [0,-10] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K107 | down | nudgeSelection | [0,1] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K108 | shift+down | nudgeSelection | [0,10] | 100 | no | yes | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L93` |
| K109 | alt+left | nudgeKeyframes | [-1] | 100 | no | yes | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L103` |
| K110 | alt+right | nudgeKeyframes | [1] | 100 | no | yes | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L103` |
| K111 | alt+shift+left | nudgeKeyframes | [-10] | 100 | no | yes | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L103` |
| K112 | alt+shift+right | nudgeKeyframes | [10] | 100 | no | yes | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L103` |
| K113 | space | play | [] | 100 | no | no | yes | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L111` |
| K114 | home | gotoStart | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L112` |
| K115 | end | gotoEnd | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L112` |
| K116 | shift+home | gotoWorkIn | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L113` |
| K117 | shift+end | gotoWorkOut | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L113` |
| K118 | pageup | prevFrame | [] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L114` |
| K119 | pagedown | nextFrame | [] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L114` |
| K120 | backspace | delete | [] | 100 | no | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L115` |
| K121 | delete | delete | [] | 100 | no | no | no | W075 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L115` |
| K122 | escape | deselect | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L35` |
| K123 | shift+pageup | stepFrames | [-10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K124 | shift+pagedown | stepFrames | [10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K125 | cmd+left | prevFrame | [] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L114` |
| K126 | cmd+right | nextFrame | [] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L114` |
| K127 | cmd+shift+left | stepFrames | [-10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K128 | cmd+shift+right | stepFrames | [10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K129 | ctrl+left | timeline.adjacentKeyframe:prev | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L124` |
| K130 | ctrl+right | timeline.adjacentKeyframe:next | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L125` |
| K131 | ctrl+shift+left | stepFrames | [-10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K132 | ctrl+shift+right | stepFrames | [10] | 100 | no | yes | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L121` |
| K133 | j | prevVisibleEvent | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L133` |
| K134 | k | nextVisibleEvent | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L134` |
| K135 | shift+j | timeline.adjacentKeyframe:prev | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L124` |
| K136 | shift+k | timeline.adjacentKeyframe:next | [] | 100 | no | no | no | W051 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L125` |
| K137 | i | gotoLayerIn | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L137` |
| K138 | o | gotoLayerOut | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L138` |
| K139 | [ | moveLayerIn | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L139` |
| K140 | ] | moveLayerOut | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L140` |
| K141 | alt+[ | trimIn | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L141` |
| K142 | alt+] | trimOut | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L142` |
| K143 | alt+home | moveLayerInToStart | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L143` |
| K144 | alt+end | moveLayerOutToEnd | [] | 100 | no | no | no | W056 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L144` |
| K145 | . | zoomIn | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L73` |
| K146 | , | zoomOut | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L77` |
| K147 | / | actualSize | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L79` |
| K148 | shift+/ | fitComposition | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L78` |
| K149 | shift+f3 | graph | [] | 100 | no | no | no | W061 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L151` |
| K150 | f9 | easyEase | [] | 100 | no | no | no | W061 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L154` |
| K151 | shift+f9 | easyEaseIn | [] | 100 | no | no | no | W061 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L155` |
| K152 | cmd+shift+f9 | easyEaseOut | [] | 100 | no | no | no | W061 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L156` |
| K153 | ctrl+shift+f9 | easyEaseOut | [] | 100 | no | no | no | W061 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L156` |
| K154 | v | toolSelect | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K155 | shift+v | toolSelect | [] | 100 | no | no | no | W046 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K156 | h | toolHand | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K157 | shift+h | toolHand | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K158 | z | toolZoom | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K159 | shift+z | toolZoom | [] | 100 | no | no | no | W045 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L161` |
| K160 | w | toolRotate | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K161 | shift+w | toolRotate | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K162 | y | toolAnchor | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K163 | shift+y | toolAnchor | [] | 100 | no | no | no | W047 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K164 | q | toolShape | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K165 | shift+q | toolShape | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K166 | g | toolPen | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K167 | shift+g | toolPen | [] | 100 | no | no | no | W048 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L162` |
| K168 | b | workIn | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L163` |
| K169 | shift+b | workIn | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L163` |
| K170 | n | workOut | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L163` |
| K171 | shift+n | workOut | [] | 100 | no | no | no | W052 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L163` |
| K172 | p | timeline.revealProperty:p | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K173 | shift+p | timeline.revealProperty:p | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K174 | s | timeline.revealProperty:s | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K175 | shift+s | timeline.revealProperty:s | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K176 | r | timeline.revealProperty:r | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K177 | shift+r | timeline.revealProperty:r | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K178 | t | timeline.revealProperty:t | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K179 | shift+t | timeline.revealProperty:t | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K180 | a | timeline.revealProperty:a | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K181 | shift+a | timeline.revealProperty:a | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K182 | u | timeline.revealProperty:u | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K183 | shift+u | timeline.revealProperty:u | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K184 | m | timeline.revealProperty:m | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K185 | shift+m | timeline.revealProperty:m | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K186 | f | timeline.revealProperty:f | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K187 | shift+f | timeline.revealProperty:f | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K188 | e | timeline.revealProperty:e | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K189 | shift+e | timeline.revealProperty:e | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K190 | l | timeline.revealProperty:l | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K191 | shift+l | timeline.revealProperty:l | [true] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L169` |
| K192 | cmd+&#96; | timeline.revealAll | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L175` |
| K193 | ctrl+&#96; | timeline.revealAll | [] | 100 | no | no | no | W060 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L175` |
| K194 | escape | blurField | [] | 50 | yes | no | no | W010 | `v1.2.0:apps/desktop/src/extensions/keymap-default/keymap.ts:L183` |
| K195 | enter | text.editSelected | [] | 0 | no | no | no | W049 | `v1.2.0:apps/desktop/src/extensions/viewer/index.ts:L50` |

## Native menu equivalents

These are dispatch paths in addition to the registry. CommandOrControl denotes the platform primary key. Native system role defaults remain system-owned; reproduce their user action through AppKit, not Electron internals. Menu-only Close/Next/Previous Tab and New/Close Window must not disappear because they have no keymap-default entry.

| ID | Accelerator as shipped | Target | Label | Source |
|---|---|---|---|---|
| M001 | CommandOrControl+= | zoomIn | Zoom In | `v1.2.0:apps/desktop/src/main/menu.ts:L69` |
| M002 | CommandOrControl+- | zoomOut | Zoom Out | `v1.2.0:apps/desktop/src/main/menu.ts:L70` |
| M003 | CommandOrControl+1 | actualSize | Actual Size | `v1.2.0:apps/desktop/src/main/menu.ts:L71` |
| M004 | CommandOrControl+0 | fitComposition | Fit Composition | `v1.2.0:apps/desktop/src/main/menu.ts:L72` |
| M005 | CommandOrControl+Shift+H | toggleLayerControls | Show/Hide Layer Controls | `v1.2.0:apps/desktop/src/main/menu.ts:L73` |
| M006 | CommandOrControl+, | settings | Settings… | `v1.2.0:apps/desktop/src/main/menu.ts:L91` |
| M007 | CommandOrControl+Alt+N | newProject | New Project | `v1.2.0:apps/desktop/src/main/menu.ts:L104` |
| M008 | CommandOrControl+O | open | Open Project… | `v1.2.0:apps/desktop/src/main/menu.ts:L112` |
| M009 | CommandOrControl+W | closeTab | Close Tab | `v1.2.0:apps/desktop/src/main/menu.ts:L114` |
| M010 | CommandOrControl+S | save | Save Project | `v1.2.0:apps/desktop/src/main/menu.ts:L116` |
| M011 | CommandOrControl+Shift+S | saveAs | Save Project As… | `v1.2.0:apps/desktop/src/main/menu.ts:L117` |
| M012 | CommandOrControl+I | import | Import Media… | `v1.2.0:apps/desktop/src/main/menu.ts:L119` |
| M013 | CommandOrControl+E | export | Export… | `v1.2.0:apps/desktop/src/main/menu.ts:L123` |
| M014 | CommandOrControl+Z | contextUndo | Undo | `v1.2.0:apps/desktop/src/main/menu.ts:L137` |
| M015 | CommandOrControl+Shift+Z | contextRedo | Redo | `v1.2.0:apps/desktop/src/main/menu.ts:L138` |
| M016 | CommandOrControl+X | contextCut | Cut | `v1.2.0:apps/desktop/src/main/menu.ts:L140` |
| M017 | CommandOrControl+C | contextCopy | Copy | `v1.2.0:apps/desktop/src/main/menu.ts:L141` |
| M018 | CommandOrControl+V | contextPaste | Paste | `v1.2.0:apps/desktop/src/main/menu.ts:L142` |
| M019 | CommandOrControl+A | contextSelectAll | Select All | `v1.2.0:apps/desktop/src/main/menu.ts:L143` |
| M020 | CommandOrControl+D | duplicate | Duplicate Layers | `v1.2.0:apps/desktop/src/main/menu.ts:L145` |
| M021 | CommandOrControl+Shift+D | split | Split at Playhead | `v1.2.0:apps/desktop/src/main/menu.ts:L146` |
| M022 | CommandOrControl+] | bringForward | Bring Forward | `v1.2.0:apps/desktop/src/main/menu.ts:L148` |
| M023 | CommandOrControl+[ | sendBackward | Send Backward | `v1.2.0:apps/desktop/src/main/menu.ts:L149` |
| M024 | CommandOrControl+Shift+] | bringToFront | Bring to Front | `v1.2.0:apps/desktop/src/main/menu.ts:L150` |
| M025 | CommandOrControl+Shift+[ | sendToBack | Send to Back | `v1.2.0:apps/desktop/src/main/menu.ts:L151` |
| M026 | CommandOrControl+N | newComposition | New Composition… | `v1.2.0:apps/desktop/src/main/menu.ts:L157` |
| M027 | CommandOrControl+Alt+K | compositionSettings | Composition Settings… | `v1.2.0:apps/desktop/src/main/menu.ts:L158` |
| M028 | CommandOrControl+Shift+C | precompose | Pre-compose… | `v1.2.0:apps/desktop/src/main/menu.ts:L160` |
| M029 | Control+Tab | nextTab | Show Next Tab | `v1.2.0:apps/desktop/src/main/menu.ts:L171` |
| M030 | Control+Shift+Tab | previousTab | Show Previous Tab | `v1.2.0:apps/desktop/src/main/menu.ts:L172` |
| M031 | CommandOrControl+Shift+N | newWindow | New Window | `v1.2.0:apps/desktop/src/main/menu.ts:L108` |
| M032 | CommandOrControl+Shift+W | closeWindow | Close Window | `v1.2.0:apps/desktop/src/main/menu.ts:L128` |
| M033 | Command+H | hide | Hide | `v1.2.0:apps/desktop/src/main/menu.ts:L95` |
| M034 | Command+Alt+H | hideOthers | Hide Others | `v1.2.0:apps/desktop/src/main/menu.ts:L96` |
| M035 | Command+Q | quit | Quit | `v1.2.0:apps/desktop/src/main/menu.ts:L98` |

Other native roles in the tag: About, Services, togglefullscreen, Minimize, Zoom and Bring All to Front; development-only Reload/Toggle Developer Tools are host diagnostics. Their default accelerators are delegated to the system rather than hardcoded keymap entries. There is no shipped Layer menu, Help menu, explicit Show All, or Edit > Delete menu row.

## Focus-local keyboard routes

Each row adds an acceptance variant to its workflow: focus the named surface, exercise every listed key, verify the local result, then verify no unrelated editor command fired. These handlers are not registry bindings; native focus/input controls may implement them directly. Enter/Space button defaults and ordinary text editing remain native. Mouse-modifier gestures are covered in the workflow inventory.

| Focus/context | Keys | Expected result | Workflow | Tag source |
|---|---|---|---|---|
| Source monitor | Left; Right; Home; End; Escape | Seek -5/+5 seconds or start/end on source slider; Escape closes active preview. | W021 | `v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts` |
| Canvas text | Escape; Cmd/Ctrl+Enter; Tab; Cmd/Ctrl+A; Cmd/Ctrl+B; Cmd/Ctrl+I; Up/Down (+Shift) | Commit; commit; suppress tab; select text; toggle bold/italic; move/extend caret vertically. Plain Enter/newline and native editing stay local. | W049 | `v1.2.0:apps/desktop/src/extensions/viewer/text-editor.ts` |
| Canvas/timeline/parent pickwhip drag | Escape | Cancel the owning drag and restore preview state. | W046 | `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts` |
| Timeline property and name drafts | Enter; Escape | Commit or cancel field; stop propagation to editor. | W055 | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| Timeline snap modifier | Shift keyDown/keyUp | Update directional snapping during the ongoing gesture, not only at pointer-down. | W054 | `v1.2.0:apps/desktop/src/extensions/timeline/timeline.ts` |
| Composition tabs | Enter; Space | Open focused composition tab. | W062 | `v1.2.0:apps/desktop/src/extensions/timeline/comp-tabs.ts` |
| Caption text edit | Enter; Shift+Enter; Escape | Commit; newline; cancel. | W067 | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |
| Caption list/search | Up; Down; Enter; Backspace; Delete; Escape | Move selection; edit; delete selected cues; clear/blur search; Down from search enters first visible cue. | W067 | `v1.2.0:apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte` |
| Effect rows | Up; Down; Alt+Up; Alt+Down; Cmd/Ctrl+C; Cmd/Ctrl+V; Backspace; Delete | Move/extend effect selection, reorder, copy/paste/delete effects before layer shortcuts. | W064 | `v1.2.0:apps/desktop/src/extensions/inspector/EffectsSection.svelte` |
| Animator name/smoothing popover | Enter; Escape | Commit rename or dismiss/cancel and restore local focus. | W066 | `v1.2.0:apps/desktop/src/extensions/inspector/TextAnimators.svelte` |
| Mod row | Enter; Space; Shift+F10 | Activate row; open context menu. | W037 | `v1.2.0:apps/desktop/src/extensions/mods/ModRow.svelte` |
| Toolbar dropdown trigger | Down | Open tool dropdown. | W048 | `v1.2.0:apps/desktop/src/extensions/toolbar/index.ts` |
| 3D navigation | F; Home; Escape | Frame selection/all without modifier; cancel active gesture or return to selection navigation. | W050 | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/navigation.ts` |
| 3D gizmo | Escape | Cancel transform. | W050 | `v1.2.0:apps/desktop/src/renderer/src/core/scene3d/gizmo.ts` |
| Numeric readout | Enter; F2; Up; Down; Shift+Up; Shift+Down | Begin text edit or change by one/ten steps. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| Numeric draft | Enter; Escape; Up; Down (+Shift) | Commit finite arithmetic/relative draft; cancel; update draft by one/ten steps. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/NumField.svelte` |
| Point pad | Left; Right; Up; Down (+Shift); Home | Per-axis step, ×10 with Shift; reset both axes to clamped zero. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/PointField.svelte` |
| Ramp stop/midpoint | Left; Right (+Shift); Home; End | Move ±1/±10; stop endpoints 0/100, midpoint endpoints 1/99. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/RampField.svelte` |
| Slider/dial | Left; Down; Right; Up (+Shift); Home; End | Step down/up, Shift ×10, minimum/maximum. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/SliderField.svelte` |
| Toggle | Left; Up; Right; Down; Space; Enter | Set false/true or toggle; disabled control does not write. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/ToggleField.svelte` |
| TextField | Enter; Escape | Blur/commit; cancel live edit. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/TextField.svelte` |
| Font picker | Down; Up; Enter; Space; Escape; Tab | Navigate results; choose first/focused face; close and restore trigger focus. | W066 | `v1.2.0:apps/desktop/src/renderer/src/controls/FontField.svelte` |
| Color picker | arrows (+Shift); Enter; Escape; Tab/Shift+Tab | Adjust focused color axis; commit slider or parse text; cancel preview; trap focus only in popover mode. | W074 | `v1.2.0:apps/desktop/src/renderer/src/controls/ColorField.svelte` |
| Fill picker | Escape; Tab/Shift+Tab | Cancel preview or cycle contained focus. | W074 | `v1.2.0:apps/desktop/src/renderer/src/controls/FillField.svelte` |
| Segmented control | Left; Right; Up; Down; Home; End | Navigate enabled options using the control map. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/Segmented.svelte` |
| Enhanced select trigger/menu | Down; Up; Enter; Space; Home; End; Escape; Tab; printable typeahead | Open/navigate/choose/dismiss options; a data-global-space-shortcut trigger preserves global Space. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/select/menu.ts` |
| Popover menu | arrows; Home; End; Enter; Space; Escape; Tab; printable typeahead | Navigate/pick/close within local menu. | W017 | `v1.2.0:apps/desktop/src/renderer/src/controls/popover-menu.ts` |
| BackTitle | Enter; Space | Activate back action if present. | W073 | `v1.2.0:apps/desktop/src/renderer/src/controls/BackTitle.svelte` |
| Legacy numeric/text field | Enter; Escape; Up/Down (+Shift) | Commit, cancel or step without triggering editor commands. | W073 | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/controls.ts` |
| Legacy/workspace curve | Enter; Space; arrows (+Shift) | Switch active handle and adjust its coordinates. | W065 | `v1.2.0:apps/desktop/src/renderer/src/legacy/core/workspace.ts` |
| Dock splitter | Enter; arrows along splitter axis | Toggle dock collapse or transfer size in 10-pixel steps. | W014 | `v1.2.0:apps/desktop/src/renderer/src/layout/Splitter.svelte` |
| Layout drag | Escape | Cancel panel placement. | W014 | `v1.2.0:apps/desktop/src/renderer/src/layout/drag.ts` |
| Library modal | Escape; Tab/Shift+Tab | Return/close and contain focus while open. | W016 | `v1.2.0:apps/desktop/src/renderer/src/library/LibraryScreen.svelte` |
| Mixer fader | Up; Right; Down; Left (+Shift); keyUp | Apply keyboard nudge and finish key gesture on release. | W068 | `v1.2.0:apps/desktop/src/renderer/src/mixer/Fader.svelte` |
| Mixer level draft | Enter; Escape | Open/commit or cancel exact level edit. | W068 | `v1.2.0:apps/desktop/src/renderer/src/mixer/LevelReadout.svelte` |
| Menu overlay | Down; Up; Left; Right; Home; End; Enter; Space; Escape; Tab | Navigate ordinary/curve-grid rows; choose; close, with Tab allowed to leave. | W017 | `v1.2.0:apps/desktop/src/renderer/src/overlays/Menu.svelte` |
| Modal | Escape; Tab/Shift+Tab | Dismiss if allowed, otherwise trap focus and restore it on close. | W017 | `v1.2.0:apps/desktop/src/renderer/src/overlays/Modal.svelte` |
| Palette | Down; Up; Home; End; Enter; Escape; Tab | Choose/execute eligible result or dismiss and restore focus. | W017 | `v1.2.0:apps/desktop/src/renderer/src/overlays/Palette.svelte` |
| Media selection | Cmd/Ctrl+A; arrows; Home; End; Enter; Space; Delete; Backspace; Escape | Select all/ranges; wrap item focus; add/play/delete/clear; nested action buttons keep native Enter/Space. | W018 | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| Media folder/comp/rename | Enter; Delete; Backspace; Escape | Open folder/comp, request delete, clear selection; folder rename Enter commits/Escape cancels. | W018 | `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte` |
| Easing panel | Cmd/Ctrl+C; Cmd/Ctrl+V; Cmd/Ctrl+Shift+V; Cmd/Ctrl+Z; Cmd/Ctrl+Shift+Z | Copy curve, paste easing/values, local graph Undo/Redo. | W065 | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| Easing library groups | Left; Right; Home; End | Cycle/select group. | W065 | `v1.2.0:apps/desktop/src/renderer/src/panels/EasingPanel.svelte` |
| Curve editor | Enter; Space; arrows (+Shift); Backspace; Delete; Escape; Tab | Cycle handle, adjust coordinates, remove interior point, cancel drag, allow Tab to leave. | W065 | `v1.2.0:apps/desktop/src/renderer/src/panels/easing/CurveEditor.svelte` |
| Generated curve control | Enter; Space; arrows (+Shift) | Switch active handle and change coordinates. | W073 | `v1.2.0:apps/desktop/src/renderer/src/panels/generated/CurveControl.svelte` |
| Effects browser | arrows; Home; End; Enter; Escape | Navigate group/items, apply effect; Escape clears/closes search. | W064 | `v1.2.0:apps/desktop/src/renderer/src/panels/FxBrowserPanel.svelte` |
| Repair agent | arrows; Enter; Shift+Enter; Escape | Navigate choices, submit single-line action, retain newline or blur field. | W037 | `v1.2.0:apps/desktop/src/renderer/src/panels/RepairAgent.svelte` |
| Agent popover | Escape | Close open popover if key was not already handled. | W025 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent-shell.ts` |
| Composer | Enter; Shift+Enter; Cmd/Ctrl+A; Cmd/Ctrl+V; Escape; repeat Enter; IME | Send once, newline, select draft, native paste, dismiss local command UI; repeats and composition do not submit. | W026 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte` |
| Composer slash menu | Down; Up; Enter; Tab; Escape | Wrap selection, insert selected command, dismiss. | W026 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte` |
| Inline attachment editor | Backspace; Delete; Cmd/Ctrl+Z; Cmd/Ctrl+Shift+Z | Arm then remove token; attachment-preserving local Undo/Redo. | W026 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/inline-prompt.ts` |
| Thread picker | Up; Down; Enter; Space; Escape; Tab | Navigate/choose thread or close picker. | W027 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/ThreadPicker.svelte` |
| Model picker | Escape | Blur local select. | W024 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/ModelPicker.svelte` |
| Question response | Enter; Shift+Enter; IME | Submit only unshifted non-composing Enter. | W028 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/QuestionCard.svelte` |
| Visual reply dialog | Escape | Close expanded visual and restore attachment focus. | W029 | `v1.2.0:apps/desktop/src/renderer/src/panels/agent/VisualReply.svelte` |
| Shader editor | Tab; Shift+Tab; Escape; Cmd/Ctrl+Enter | Indent/outdent; arm Tab exit; compile; subsequent Tab exits. | W078 | `v1.2.0:apps/desktop/src/renderer/src/panels/shader/ShaderEditor.svelte` |
| Settings | Up; Down; Home; End; Escape; exact Cmd/Ctrl+, | Navigate visible sections or close; no editor-key leakage. | W012 | `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte` |
| Extension settings | Up; Down; Home; End; Escape | Navigate extension categories/details and close detail. | W037 | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/extension-settings.ts` |
| Project tab/rename | F2; Left; Right; Home; End; Enter; Space; Backspace; Delete; Escape | Rename, wrap tab focus, activate/close tab; rename Enter commits/Escape cancels. | W002 | `v1.2.0:apps/desktop/src/renderer/src/shell/Titlebar.svelte` |
| Task pill | Escape | Close tasks popover and restore trigger focus. | W017 | `v1.2.0:apps/desktop/src/renderer/src/shell/TaskPill.svelte` |
| Store picker | Up; Down; Enter; Space; Escape; Tab | Navigate/choose or dismiss search picker. | W036 | `v1.2.0:apps/desktop/src/renderer/src/store/SearchPicker.svelte` |
| Store icon cropper | arrows; +; =; -; Enter; Escape | Pan crop center, zoom, confirm or cancel. | W036 | `v1.2.0:apps/desktop/src/renderer/src/store/IconCropper.svelte` |
| Store detail | Escape | Return from detail/profile under current query/page conditions. | W036 | `v1.2.0:apps/desktop/src/renderer/src/store/StoreScreen.svelte` |
| Projects screen | Escape | Dismiss nested project screen action. | W006 | `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/projects.ts` |

## Property-reveal sequences

Single keys and their Shift variants use the existing K rows above; these are stateful repeat-tap meanings, not extra registry entries. Tap the same key twice within 300 ms with the same Shift state and same selected layers. Shift toggles the requested rows against existing reveal; no selection targets the whole composition, except M is an immediate global disclosure toggle. None of these changes animation data.

| Key | Single tap | Double tap |
|---|---|---|
| P | Position | Paint / Puppet / Roto Brush property groups |
| S | Scale | Selected property or selected-key properties |
| R | Rotation / Orientation | Source time / time remapping |
| T | Opacity | Mask opacity |
| A | Anchor point | Material Options |
| U | Animated or expression properties | Also include properties changed from defaults |
| M | Mask path/geometry controls; no selection toggles all strips/groups | All mask properties when selected; no selection keeps global toggle |
| F | Mask feather | Properties of missing effect definitions |
| E | Effect parameters | Expressions |
| L | Audio gain | Hide property rows (waveform remains on audio strip) |

Acceptance: single-tap reveal/toggle closed, double-tap within/outside threshold, Shift add/remove, same-key selection change, empty composition, selected groups and global M, with no document edit or Undo entry. Source: `v1.2.0:apps/desktop/src/extensions/timeline/property-reveal.ts` and `v1.2.0:apps/desktop/src/renderer/src/kernel/property-shortcuts.ts`.

## Post-v1.2.0 bindings

On main, plain backtick → maximizePanel is added to the 194 default rows. On the native-engine branch, the later priority-90 rows are j → shuttleBackward(false), k → shuttleStop, l → shuttleForward(false), ctrl+j → shuttleBackward(true), ctrl+shift+space → playAround. They are **not in v1.2.0**. While unsupported/paused (except the forced Control-J start), these decline and let shipped J/K event navigation or L property reveal run. Native cutover must deliver the explicitly required shuttle addition without replacing the shipped fallthrough behavior. See W079.
