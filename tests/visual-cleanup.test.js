const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const workspace = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
const tokens = fs.readFileSync(path.join(root, 'css/tokens.css'), 'utf8');
const appCss = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
const app = fs.readFileSync(path.join(root, 'js/app.js'), 'utf8');

test('major section borders have a dedicated perceptual weight in both themes', () => {
  const light = tokens.slice(tokens.indexOf(':root{'), tokens.indexOf(':root[data-density'));
  const dark = tokens.slice(tokens.indexOf(':root[data-theme="dark"]{', tokens.indexOf('/* ── dark theme')));
  assert.match(light, /--section-line:rgba\(15,15,20,\.13\)/);
  assert.match(dark, /--section-line:rgba\(255,255,255,\.08\)/);
  assert.match(appCss, /\.panel\s*\{[^}]*border:1px solid var\(--section-line\)/s);
  assert.match(appCss, /\.panel > header\s*\{[^}]*border-bottom:1px solid var\(--section-line\)/s);
  assert.match(appCss, /#tl-head\s*\{[^}]*border-bottom:1px solid var\(--section-line\)/s);
});

test('timeline annotation markers and labels are absent without affecting keyframes', () => {
  const ruler = timeline.slice(timeline.indexOf('function drawRuler'), timeline.indexOf('function fmtRuler'));
  assert.doesNotMatch(ruler, /markers|m\.name/);
  assert.doesNotMatch(timeline, /Add marker at playhead|clicking a marker diamond|proj\.markers\.forEach/);
  assert.doesNotMatch(fs.readFileSync(path.join(root, 'js/ui/shortcuts.js'), 'utf8'), /def\('marker'|go\('marker'\)/);
  assert.match(timeline, /p\.prop\.kf\.forEach/, 'real keyframes remain part of timeline navigation');
});

test('Design workspace uses the timeline and has no retired Generative panel', () => {
  const design = workspace.slice(workspace.indexOf("id: 'design'"), workspace.indexOf("id: 'gradient'"));
  assert.doesNotMatch(design, /p\('layers'/);
  /* no assistant or retired Generative surface is docked */
  assert.doesNotMatch(design, /p\('chat'/);
  assert.doesNotMatch(design, /p\('library'/);
  assert.match(design, /p\('assets', \{ size: 190 \}\), p\('fxbrowser', \{ flex: true \}\)/);
});

test('Gradient is a clean shared preset instead of app-only saved state', () => {
  const gradient = workspace.slice(workspace.indexOf("id: 'gradient'"), workspace.indexOf("id: 'animate'"));
  assert.match(gradient, /name: 'Gradient', builtin: true/);
  assert.match(gradient, /dock\('center', \[p\('viewer', \{ flex: true \}\), p\('timeline', \{ size: 300 \}\)\]\)/);
  assert.doesNotMatch(gradient, /p\('layers'/);
  assert.match(gradient, /label: 'End color'.*target: '\$composition'.*path: 'composition\.background\.endColor'.*def: '#34144F'/);
  assert.match(gradient, /label: 'Midpoint'.*target: '\$composition'.*path: 'composition\.background\.midpoint'/);
  assert.doesNotMatch(gradient, /param: 'Gradient/, 'gradient controls edit real composition source instead of inert parameters');
});

test('non-editable chrome cannot be selected while text editors remain selectable', () => {
  assert.match(appCss, /body \*\{-webkit-user-select:none;user-select:none\}/);
  assert.match(appCss, /textarea,[\s\S]*\[contenteditable\][\s\S]*-webkit-user-select:text;user-select:text/);
});

test('the Composition surface is square, borderless, and shadowless while surrounding chrome keeps elevation', () => {
  const stage = appCss.match(/#stage-inner\{([^}]*)\}/)?.[1] || '';
  assert.match(stage, /border-radius:0/);
  assert.match(stage, /corner-shape:round/, 'the preview opts out of UI superellipse smoothing');
  assert.match(stage, /overflow:hidden/);
  assert.match(stage, /box-shadow:none/);
  assert.match(stage, /outline:0/, 'the composition color reaches the edge without a dark outline');
  assert.doesNotMatch(stage, /outline-offset|outline:1px|border:/);
  assert.match(appCss, /#library-screen\{[^}]*box-shadow:var\(--shadow-float\)/s, 'unrelated elevation remains intact');
});

test('one Library control replaces duplicate global commands without hiding unique actions', () => {
  const titlebar = app.slice(app.indexOf('right.append('), app.indexOf("PM.bus.on('workspaces', paintTabs)"));
  assert.match(titlebar, /Library · Sections and Workspaces/);
  assert.doesNotMatch(titlebar, /button\('plus', 'New layer'/);
  assert.doesNotMatch(titlebar, /button\('wand', 'New shader layer'/);
  assert.doesNotMatch(titlebar, /button\('export', 'Export'/);
  assert.doesNotMatch(titlebar, /button\('gear', 'Workspace definition'/);
  assert.match(fs.readFileSync(path.join(root, 'js/ui/library.js'), 'utf8'), /Edit validated definition….*PM\.WS\.editJSON/s,
    'validated workspace definition editing remains reachable from Library');
  assert.match(fs.readFileSync(path.join(root, 'js/ui/toolbar.js'), 'utf8'), /PM\.cmd\('newShader'\)/,
    'shader creation remains reachable from the canonical tool strip');
  assert.match(fs.readFileSync(path.join(root, 'js/ui/viewer.js'), 'utf8'), /PM\.Export\.dialog\(\)/,
    'export remains reachable from the composition surface');
  assert.doesNotMatch(fs.readFileSync(path.join(root, 'js/ui/toolbar.js'), 'utf8'), /title: 'Snapping \(S\)'/,
    'the global snapping duplicate is removed while the timeline owns its control');
});

test('tool controls sit on the right in a tabs-like smoothed group', () => {
  assert.match(app, /bar\.insertBefore\(el, bar\.querySelector\('#tb-right'\)\)/);
  assert.match(appCss, /#toolbar-strip\{[^}]*background:var\(--bg-sunken\)[^}]*border-radius:10px/s);
  assert.match(appCss, /#toolbar-strip[^}]*-webkit-app-region:no-drag/s);
  assert.match(app, /el\.hidden = !!\(PM\.ProjectsScreen && PM\.ProjectsScreen\.isOpen\)/,
    'Home hides editor-only tools');
  assert.match(appCss, /#toolbar-strip\[hidden\]\{display:none\}/);
});

test('top-level Settings owns appearance while Undo and Redo remain keyboard commands', () => {
  const titlebar = app.slice(app.indexOf('right.append('), app.indexOf("PM.bus.on('workspaces', paintTabs)"));
  assert.match(titlebar, /button\('gear', 'Settings', openSettings\)/);
  assert.doesNotMatch(titlebar, /button\('undo', 'Undo'/);
  assert.doesNotMatch(titlebar, /button\('redo', 'Redo'/);
  assert.doesNotMatch(titlebar, /Toggle light \/ dark appearance|themeButton/);
  assert.match(app, /PM\.SettingsUI = \{ open: openSettings \}/);
  assert.match(app, /aria-label': 'Appearance'/);
  assert.match(app, /appearance\.onchange = \(\) => PM\.theme\.apply\(appearance\.value\)/);
  assert.match(app, /PM\.store\.set\('theme', t\)/, 'appearance preference stays persisted');
  const shortcuts = fs.readFileSync(path.join(root, 'js/ui/shortcuts.js'), 'utf8');
  assert.match(shortcuts, /def\('undo'/);
  assert.match(shortcuts, /def\('redo'/);
});

test('the Composition color picker is reachable and its color surfaces are borderless', () => {
  const controls = fs.readFileSync(path.join(root, 'js/ui/controls.js'), 'utf8');
  assert.match(controls, /button\.color-field/);
  assert.match(controls, /function openColorPicker\(initial, apply, label\)/);
  assert.match(controls, /input\.color-hex/);
  assert.match(controls, /once\(set, value, opt, opt\.label \|\| 'Color'\)/,
    'Apply crosses the shared undoable edit boundary');
  assert.doesNotMatch(controls, /type: 'color'[^\n]*width: 0|inp\.click\(\)/,
    'the broken zero-size native proxy is gone');
  const dialogPreview = appCss.match(/\.color-dialog-preview\{([^}]*)\}/)?.[1] || '';
  const colorChoice = appCss.match(/\.color-choice\{([^}]*)\}/)?.[1] || '';
  const inlineSwatch = appCss.match(/\.sw\{([^}]*)\}/)?.[1] || '';
  for (const surface of [dialogPreview, colorChoice, inlineSwatch]) {
    assert.doesNotMatch(surface, /padding:|border:|var\(--line-2\)/,
      'color is painted directly to the edge without a dark frame');
  }
  assert.match(dialogPreview, /background:var\(--sw-color\)/);
  assert.match(colorChoice, /background:var\(--sw-color\)/);
  assert.match(inlineSwatch, /background:var\(--sw-fill,var\(--sw-color,#808080\)\)/);
  assert.match(appCss, /\.color-choice\.on\{box-shadow:0 0 0 2px var\(--accent\)\}/,
    'only the selected preset gets an intentional accent ring');
});

test('custom fill picker supports solid and gradient editing without a native picker', () => {
  const controls = fs.readFileSync(path.join(root, 'js/ui/controls.js'), 'utf8');
  const inspector = fs.readFileSync(path.join(root, 'js/ui/inspector.js'), 'utf8');
  const model = fs.readFileSync(path.join(root, 'js/core/model.js'), 'utf8');
  const compositor = fs.readFileSync(path.join(root, 'js/gl/compositor.js'), 'utf8');
  assert.match(inspector, /PM\.fillField/);
  assert.match(controls, /\['solid', 'Solid'\].*\['linear', 'Linear'\].*\['radial', 'Radial'\].*\['none', 'None'\]/s);
  assert.match(controls, /role: 'group', 'aria-label': 'Fill type'/);
  assert.match(controls, /Add stop/);
  assert.match(controls, /Remove stop/);
  assert.match(controls, /fill-sv/);
  assert.match(controls, /fill-hue/);
  assert.match(controls, /Saturation and brightness/);
  assert.match(controls, /Color swatches/);
  assert.match(controls, /button\.fill-channels-toggle/);
  assert.match(controls, /channelGrid\.hidden = !channelGrid\.hidden/,
    'advanced color channels stay available without crowding the default picker');
  assert.match(controls, /PM\.Color = \{ normalizeHex, rgbToHex, hexToRgb, rgbToHsv, hsvToRgb \}/,
    'hex, RGB, and HSB fields share one synchronized conversion path');
  assert.match(appCss, /\.fill-picker\{[^}]*width:400px[^}]*max-height:min\(560px/,
    'the picker stays compact enough for the editor');
  assert.match(appCss, /\.fill-color-main\{height:164px[^}]*grid-template-columns:1fr 18px/,
    'the picker keeps a useful 2D field and a slim vertical hue rail');
  assert.match(appCss, /\.fill-preview\{height:10px/,
    'the gradient preview is a quiet rail instead of a second oversized color surface');
  assert.match(appCss, /\.fill-channels\[hidden\]\{display:none\}/,
    'RGB and HSB channels are collapsed by default');
  assert.match(appCss, /\.fill-color-workbench\[hidden\],\.fill-preview\[hidden\],\.fill-stops\[hidden\]\{display:none!important\}/,
    'mode-specific sections stay hidden even though their normal layout uses flex');
  assert.match(appCss, /\.fill-picker :is\([^}]*:focus-visible\{outline:2px solid color-mix\(in srgb,var\(--accent\)/,
    'picker focus uses the app accent instead of the browser black outline');
  const svIndicator = appCss.match(/\.fill-sv i\{([^}]*)\}/)?.[1] || '';
  const hueIndicator = appCss.match(/\.fill-hue i\{([^}]*)\}/)?.[1] || '';
  assert.doesNotMatch(svIndicator + hueIndicator, /0 0 0 1px #000/, 'picker indicators have no hard black ring');
  assert.match(svIndicator + hueIndicator, /0 0 0 1px var\(--accent\)/,
    'picker indicators use the same accent edge as keyboard focus');
  assert.match(controls, /if \(event\.key === 'Escape'\)/, 'custom popover is keyboard-cancelable');
  assert.match(controls, /if \(event\.key !== 'Tab'\) return/);
  assert.match(controls, /focusable\.at\(-1\)/, 'keyboard focus is trapped inside the open picker');
  assert.match(controls, /ArrowLeft.*ArrowRight.*ArrowUp.*ArrowDown/s,
    'the saturation-value field is keyboard operable');
  assert.match(controls, /applyButton\.onclick = \(\) => \{ apply\(PM\.normalizeFill\(draft\)\); close\(\); \}/,
    'draft changes reach source only after Apply');
  assert.match(model, /PM\.normalizeFill/);
  assert.match(compositor, /PM\.FRAG_BACKGROUND_FILL/);
  assert.doesNotMatch(controls, /type: 'color'/, 'the custom fill picker never invokes the native macOS color picker');
});

test('Section scrollbars are hidden without removing scrolling semantics', () => {
  assert.match(appCss, /\.panel>\.body,#library-screen>main,\.spatial-preview,\.pop-mirror,\.fill-picker-body\{scrollbar-width:none\}/);
  assert.match(appCss, /\.panel>\.body::-webkit-scrollbar[^}]*display:none/);
  assert.match(appCss, /\.panel > \.body\{[^}]*overflow:auto/s);
  assert.match(appCss, /#library-screen>main\{[^}]*overflow:auto/s);
  assert.match(appCss, /\.spatial-preview\{[^}]*overflow:auto/s);
});

test('product chrome keeps readable title casing instead of forced all-caps', () => {
  assert.doesNotMatch(appCss, /text-transform\s*:\s*uppercase/);
  const library = fs.readFileSync(path.join(root, 'js/ui/library.js'), 'utf8');
  assert.match(library, /'Sections'/);
  assert.match(library, /'Workspaces'/);
  assert.match(library, /'Looks'/);
});

test('obsolete Guides and Motion Blur Preview product controls are completely absent', () => {
  const read = rel => fs.readFileSync(path.join(root, rel), 'utf8');
  const toolbar = read('js/ui/toolbar.js');
  const viewer = read('js/ui/viewer.js');
  const engine = read('js/core/engine.js');
  const workspaceSource = read('js/core/workspace.js');
  for (const text of [toolbar, viewer]) {
    assert.doesNotMatch(text, /Guides & safe areas/i);
    assert.doesNotMatch(text, /Motion blur preview/i);
  }
  assert.doesNotMatch(engine, /PM\.guides|PM\.mblurOn/);
  assert.match(workspaceSource, /delete raw\.features\.guides/);
  assert.match(workspaceSource, /delete raw\.features\.motionBlur/);
});
