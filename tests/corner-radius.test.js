const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const css = fs.readFileSync(path.resolve(__dirname, '../css/app.css'), 'utf8');

test('nested toolbar corners follow outer radius = inner radius + inset', () => {
  assert.match(css, /#toolbar-strip\s*\{[^}]*--toolbar-control-inset:6px/s);
  assert.match(css, /border-radius:calc\(var\(--r-sm\) \+ var\(--toolbar-control-inset\)\)/);
  assert.match(css, /padding:0 calc\(var\(--toolbar-control-inset\) - 1px\)/);
});
