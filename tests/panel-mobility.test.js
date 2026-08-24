const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const root = path.resolve(__dirname, '..');
const layout = fs.readFileSync(path.join(root, 'js/ui/layout.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
const timeline = fs.readFileSync(path.join(root, 'js/ui/timeline.js'), 'utf8');
const viewer = fs.readFileSync(path.join(root, 'js/ui/viewer.js'), 'utf8');
const native = fs.readFileSync(path.join(root, 'native/main.swift'), 'utf8');

test('headless panels expose a dedicated move handle unless the surface intentionally hides it', () => {
  assert.match(layout, /button\.panel-move-handle/);
  assert.match(layout, /headless && !def\.hideMoveHandle/);
  assert.match(layout, /Move \$\{def\.title\} panel/);
  assert.match(layout, /moveHandle\.addEventListener\('pointerdown', beginMove\)/);
  assert.match(css, /\.panel-move-handle\s*\{/);
  assert.match(css, /cursor:grab/);
  assert.match(css, /\.panel-move-handle\.inline/);
  assert.match(css, /\.panel-move-handle\.inline\{[^}]*width:16px[^}]*border:0[^}]*background:transparent[^}]*box-shadow:none/s);
  assert.match(layout, /slot\.insertBefore\(moveHandle, slot\.firstChild\)/);
  assert.match(timeline, /moveSlot: '#tl-head'/, 'Timeline keeps its drag handle');
  assert.match(viewer, /hideMoveHandle: true/, 'Composition removes its footer drag grip');
});

test('every resize grip smoothly follows the cursor and fades toward both ends', () => {
  assert.match(layout, /attachSplitterHover/);
  assert.match(layout, /--splitter-hover-x/);
  assert.match(layout, /--splitter-hover-y/);
  assert.match(layout, /current \+= \(target - current\) \* \.36/,
    'cursor motion is eased continuously instead of snapping between pointer events');
  assert.match(layout, /attachSplitterHover\(s, 'x'\)/, 'horizontal grips track the cursor x position');
  assert.match(layout, /attachSplitterHover\(s, 'y'\)/, 'vertical grips track the cursor y position');
  assert.match(layout, /move: \(dx, dy, ev\)[\s\S]*trackPointer\(ev\)/,
    'the hotspot keeps following the cursor during drag');
  assert.match(css, /\.splitter::after\{[^}]*opacity:0[^}]*transition:opacity \.18s/s,
    'every resize grip is completely invisible at rest');
  assert.match(css, /\.splitter:hover::after,\.splitter\.drag::after\{opacity:1;transition-duration:\.12s\}/,
    'every grip fades in on hover or drag and fades out after leaving');
  assert.match(css, /radial-gradient\(12px 180px[\s\S]*transparent 100%/,
    'the hover glow fades fully away outside the vertical cursor hotspot');
  assert.match(css, /radial-gradient\(180px 12px[\s\S]*transparent 100%/,
    'the hover glow fades fully away outside the horizontal cursor hotspot');
  assert.match(css, /radial-gradient\(180px 12px at var\(--splitter-hover-x,50%\) 50%/,
    'horizontal opacity peaks locally beneath the cursor');
  assert.match(css, /radial-gradient\(12px 180px at 50% var\(--splitter-hover-y,50%\)/,
    'vertical opacity peaks locally beneath the cursor');
});

test('persisted canvas panels resolve their current dock before moving again', () => {
  assert.match(layout, /findPanel\(L\.ws, spec\.id\)/);
  assert.match(layout, /inst\.dock = dock/);
  assert.match(layout, /startPanelDrag\(e, current\.spec, current\.dock, el\)/);
});

test('panel dragging uses one compact destination label without workspace lines', () => {
  assert.match(layout, /span\.panel-ghost-destination/);
  assert.match(layout, /document\.body\.classList\.add\('panel-dragging'\)/);
  assert.match(layout, /document\.body\.classList\.remove\('panel-dragging'\)/);
  assert.doesNotMatch(layout, /classList\.add\(before \? 'drop-before' : 'drop-after'\)/);
  assert.doesNotMatch(layout, /dockEl\.classList\.add\('drop-into'\)/);
  assert.doesNotMatch(css, /\.dock\.drop-into\s*\{/);
  assert.doesNotMatch(css, /\.panel\.drop-(?:before|after)\s*\{/);
  assert.match(css, /\.panel-dragging \.splitter::after\{opacity:0\}/);
  assert.match(css, /\.panel-ghost\s*\{[^}]*height:32px/s);
});

test('panel placement preview is an overlay and never reflows the dock under the pointer', () => {
  assert.match(layout, /Fixed overlay:[\s\S]*never reflows the/);
  assert.doesNotMatch(layout, /dockEl\.(?:insertBefore|appendChild)\(preview/);
  assert.match(css, /\.panel-drop-preview\s*\{[^}]*position:fixed[^}]*height:4px/s);
  assert.match(css, /\.panel-drop-preview\.on/);
});

test('an interrupted titlebar drag cannot hijack a later panel gesture', () => {
  assert.match(native, /private var dragEndMonitor: Any\?/);
  assert.match(native, /matching: \[\.leftMouseDown, \.leftMouseDragged, \.leftMouseUp\]/,
    'a fresh panel mouse-down terminates any stale native window-drag monitor');
  assert.match(native, /event\.type == \.leftMouseDown \|\| event\.type == \.leftMouseUp[\s\S]*endWindowDrag\(\)/);
  assert.match(native, /addGlobalMonitorForEvents\(matching: \.leftMouseUp\)/,
    'a release delivered outside Powermove also ends native window dragging');
  assert.match(native, /if let m = dragEndMonitor \{ NSEvent\.removeMonitor\(m\); dragEndMonitor = nil \}/);
});

test('drag motion is frame-coalesced, uses generous geometry, and animates only the drop', () => {
  assert.match(layout, /requestAnimationFrame\(renderDragFrame\)/);
  assert.match(layout, /L\.buildDockDropTargets\(docks, bodyRect\)/);
  assert.match(layout, /L\.hitTestDockPlacement\(targets, ev\.clientX, ev\.clientY\)/);
  assert.match(layout, /tolerance = 28/, 'drop areas extend beyond visible dock edges');
  assert.match(layout, /capturePanelRects\(\)/);
  assert.match(layout, /node\.animate\(\[/);
  assert.match(layout, /prefers-reduced-motion: reduce/);
  assert.match(css, /@media \(prefers-reduced-motion:reduce\)/);
});

test('timeline section resize never exposes an opaque cleared canvas', () => {
  assert.doesNotMatch(timeline, /getContext\('2d',\s*\{\s*alpha:\s*false\s*\}\)/);
  assert.match(timeline, /const changed = T\.cv\.width !== width \|\| T\.cv\.height !== height/);
  assert.match(timeline, /if \(changed\) draw\(\)/);
});

test('every regular panel exposes clear options with recoverable hide and restore', () => {
  assert.match(layout, /button\.panel-options/);
  assert.match(layout, /label: 'Hide panel'.*hidePanel/s);
  assert.match(layout, /Restore \$\{PM\.PANELS\[item\.id\]\.title\}/);
  assert.match(layout, /if \(id === 'viewer'\) return false/);
  assert.match(css, /\.panel-options\{/);
});

test('pop out creates a native child window, mirrors one live panel owner, and restores on close', () => {
  const popout = layout.slice(layout.indexOf('PM.Popout ='), layout.lastIndexOf('})();'));
  assert.match(native, /javaScriptCanOpenWindowsAutomatically = true/);
  assert.match(native, /NSButton\(title: "Return to layout"/);
  assert.match(native, /target: win, action: #selector\(NSWindow\.performClose/);
  assert.match(native, /createWebViewWith configuration/);
  assert.match(native, /styleMask: \[\.titled, \.closable, \.miniaturizable, \.resizable\]/);
  assert.match(native, /win\.delegate = self/);
  assert.match(popout, /sourceHost\.appendChild\(el\)/, 'the live content has a single authoritative owner');
  assert.match(popout, /MutationObserver/, 'the child reflects authoritative source updates');
  assert.match(popout, /target\.click\(\)/, 'child controls forward to the authoritative panel');
  assert.doesNotMatch(popout, /detachedPlaceholder/, 'detached panels reserve no placeholder space');
  assert.match(layout, /L\.visibleDockPlan[\s\S]*filter\(item => !item\.dock\.hidden && item\.specs\.length\)/,
    'detached panels are removed from layout flow so neighboring content reflows');
  assert.doesNotMatch(popout, /powermove-redock/);
  assert.doesNotMatch(popout, /PM\.WS\.mutate/, 'detaching does not destroy or duplicate the layout manifest');
  assert.match(popout, /w\.closed.*PM\.Popout\.reclaim\(id\)/s);
  assert.match(popout, /if \(L\.ws\) L\.apply\(L\.ws\)/, 'close and redock rebuild from synchronized source state');
});

test('detached autosizing prompts keep compact geometry, focus, and caret while typing', () => {
  const spatial = fs.readFileSync(path.join(root, 'js/assistant/spatial.js'), 'utf8');
  const popout = layout.slice(layout.indexOf('PM.Popout ='), layout.lastIndexOf('})();'));
  assert.match(spatial, /data-autosize': 'true'/, 'assistant textareas declare visible-window autosizing');
  assert.match(spatial, /input\.closest\('\[data-popout-source\]'\)\) return/,
    'the hidden authoritative textarea is never measured as visible content');
  assert.match(popout, /sourceHost\.dataset\.popoutSource = id/);
  assert.match(popout, /resizeMirroredTextarea\(event\.target\)/,
    'the correctly sized visible textarea grows instead of the hidden source');
  assert.match(popout, /suppressInputMirror = true[\s\S]*resizeMirroredTextarea\(event\.target\);\s*return/,
    'typing updates the authoritative value without redispatching the hidden source input event');
  assert.match(popout, /records\.every\(belongsToAutosizingTextarea\)/,
    'WebKit textarea mutations do not rebuild and blur the active mirror');
  assert.match(popout, /focusedPath[\s\S]*selectionStart[\s\S]*nextActive\.focus[\s\S]*setSelectionRange/,
    'authoritative refreshes restore both focus and the exact caret range');
});

test('vertical resize handles are generous, bounded, persistent, and never start panel movement', () => {
  assert.match(css, /\.splitter\.h::before\{[^}]*inset:-6px 0[^}]*cursor:row-resize/s);
  assert.match(layout, /L\.clampPanelHeight/);
  assert.match(layout, /pairHeight/);
  assert.match(layout, /e\.stopPropagation\(\)/);
  assert.match(layout, /cancel: \(\) => \{[\s\S]*applyPanelSize/s);
});

test('custom Sections use the same native pop-out and pin-back path as regular panels', () => {
  assert.doesNotMatch(layout, /NO_POPOUT[\s\S]*custom/);
  assert.match(layout, /const def = PM\.PANELS\[id\]/);
  assert.match(layout, /sourceHost\.appendChild\(el\)/, 'one authoritative editable element is retained');
  assert.match(native, /NSButton\(title: "Return to layout"/);
});

test('native detached windows receive packaged styling without broad file access', () => {
  assert.match(native, /web\/css/);
  assert.match(native, /\["tokens\.css", "app\.css"\]/);
  assert.match(native, /WKUserScript\(source: source, injectionTime: \.atDocumentEnd/);
  assert.match(native, /location\.href==='about:blank'/);
});
