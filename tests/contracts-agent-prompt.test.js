const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const read = name => fs.readFileSync(path.join(root, name), 'utf8');
const nativeSource = read('native/main.swift');
const harnessSource = read('js/assistant/harness.js');

function functionBody(source, name) {
  const signature = source.indexOf(`func ${name}(`);
  assert.notEqual(signature, -1, `${name} must remain in native/main.swift`);
  const start = source.indexOf('{', signature);
  let depth = 1;
  let index = start + 1;
  while (index < source.length && depth) {
    if (source.startsWith('"""', index)) {
      const end = source.indexOf('"""', index + 3);
      assert.notEqual(end, -1, `unterminated multiline string in ${name}`);
      index = end + 3;
      continue;
    }
    if (source[index] === '"') {
      for (index += 1; index < source.length; index += 1) {
        if (source[index] === '\\') index += 1;
        else if (source[index] === '"') { index += 1; break; }
      }
      continue;
    }
    if (source.startsWith('//', index)) {
      index = source.indexOf('\n', index + 2);
      if (index === -1) break;
      continue;
    }
    if (source[index] === '{') depth += 1;
    else if (source[index] === '}') depth -= 1;
    index += 1;
  }
  assert.equal(depth, 0, `unterminated function body for ${name}`);
  return source.slice(start + 1, index - 1);
}

function normalizeText(value) {
  return value.replace(/\r\n?/g, '\n')
    .split('\n').map(line => line.replace(/[ \t]+$/g, '')).join('\n').trim();
}

function decodeMultilineString(raw) {
  assert.ok(raw.startsWith('\n'), 'Swift multiline literal must begin on the next line');
  const lastNewline = raw.lastIndexOf('\n');
  const indent = raw.slice(lastNewline + 1);
  assert.match(indent, /^\s*$/, 'closing multiline delimiter must be on its own line');
  return raw.slice(1, lastNewline).split('\n')
    .map(line => line.startsWith(indent) ? line.slice(indent.length) : line).join('\n');
}

function decodeQuotedString(token) {
  /* JSON and Swift share the escapes used here. Preserve Swift interpolation. */
  const interpolation = '__PM_SWIFT_INTERPOLATION__';
  const escaped = token.slice(1, -1).replace(/\\\(/g, `${interpolation}(`);
  return JSON.parse(`"${escaped}"`).replaceAll(interpolation, '\\');
}

function stringExpression(body) {
  const values = [];
  let remainder = '';
  for (let index = 0; index < body.length;) {
    if (body.startsWith('"""', index)) {
      const end = body.indexOf('"""', index + 3);
      assert.notEqual(end, -1, 'unterminated Swift multiline literal');
      values.push(decodeMultilineString(body.slice(index + 3, end)));
      index = end + 3;
      continue;
    }
    if (body[index] === '"') {
      let end = index + 1;
      while (end < body.length) {
        if (body[end] === '\\') end += 2;
        else if (body[end] === '"') { end += 1; break; }
        else end += 1;
      }
      values.push(decodeQuotedString(body.slice(index, end)));
      index = end;
      continue;
    }
    remainder += body[index];
    index += 1;
  }
  assert.match(remainder, /^\s*(?:\+\s*)*$/, 'agentInstructions must remain a static string expression');
  return normalizeText(values.join(''));
}

class SwiftLiteralParser {
  constructor(source) { this.source = source; this.index = 0; }

  whitespace() { while (/\s/.test(this.source[this.index] || '')) this.index += 1; }

  string() {
    const start = this.index;
    this.index += 1;
    while (this.index < this.source.length) {
      if (this.source[this.index] === '\\') this.index += 2;
      else if (this.source[this.index] === '"') { this.index += 1; break; }
      else this.index += 1;
    }
    return JSON.parse(this.source.slice(start, this.index));
  }

  collection() {
    this.index += 1;
    this.whitespace();
    if (this.source[this.index] === ']') { this.index += 1; return []; }

    const checkpoint = this.index;
    assert.equal(this.source[this.index], '"', 'Swift schema keys and strings must be quoted');
    this.string();
    this.whitespace();
    const dictionary = this.source[this.index] === ':';
    this.index = checkpoint;
    const result = dictionary ? {} : [];

    while (true) {
      this.whitespace();
      if (this.source[this.index] === ']') { this.index += 1; return result; }
      if (dictionary) {
        const key = this.string();
        this.whitespace();
        assert.equal(this.source[this.index], ':', `missing colon after schema key ${key}`);
        this.index += 1;
        result[key] = this.value();
      } else {
        result.push(this.value());
      }
      this.whitespace();
      if (this.source[this.index] === ',') this.index += 1;
      else assert.equal(this.source[this.index], ']', 'missing comma in Swift collection');
    }
  }

  value() {
    this.whitespace();
    const character = this.source[this.index];
    if (character === '[') return this.collection();
    if (character === '"') return this.string();
    const token = this.source.slice(this.index).match(/^(?:true|false|null|-?\d+(?:\.\d+)?)/);
    assert.ok(token, `unsupported Swift schema literal at ${this.source.slice(this.index, this.index + 20)}`);
    this.index += token[0].length;
    if (token[0] === 'true') return true;
    if (token[0] === 'false') return false;
    if (token[0] === 'null') return null;
    return Number(token[0]);
  }

  parse() {
    const value = this.value();
    this.whitespace();
    assert.equal(this.index, this.source.length, 'unexpected code after static Swift schema');
    return value;
  }
}

function agentInstructions() {
  return stringExpression(functionBody(nativeSource, 'agentInstructions'));
}

function agentResultSchema() {
  return new SwiftLiteralParser(functionBody(nativeSource, 'agentResultSchema').trim()).parse();
}

function harnessRuntime() {
  const PM = {};
  const context = vm.createContext({ window: { PM }, console, JSON, Object, Set, Map, Promise });
  vm.runInContext(harnessSource, context, { filename: 'js/assistant/harness.js' });
  return PM;
}

function sceneOperations() {
  const match = harnessSource.match(/const SCENE_OPERATIONS = new Set\(\[([\s\S]*?)\]\);/);
  assert.ok(match, 'SCENE_OPERATIONS must remain a static set');
  return [...match[1].matchAll(/'([^']+)'/g)].map(result => result[1]);
}

test('native agent instructions and result schema match their committed goldens', () => {
  const instructionsGolden = normalizeText(read('tests/fixtures/agent-instructions.golden.txt'));
  const schemaGolden = JSON.parse(read('tests/fixtures/agent-result-schema.golden.json'));
  assert.equal(agentInstructions(), instructionsGolden);
  assert.deepEqual(agentResultSchema(), schemaGolden);
});

test('agent command cleaning preserves hand edits and rejects unknown operations', () => {
  const PM = harnessRuntime();
  const cleaned = PM.AgentHarness.cleanCommand({
    type: 'set_property', target: 'title', path: 'position.x', value: 42,
    preserveHandEdits: false, unknownField: 'strip me',
  });
  assert.equal(cleaned.preserveHandEdits, true);
  assert.equal(Object.hasOwn(cleaned, 'unknownField'), false);

  const replaced = PM.AgentHarness.cleanCommand({
    type: 'replace_keyframes', target: 'title', path: 'opacity',
    keyframes: Array.from({ length: 85 }, (_, index) => ({ time: index / 30, value: index })),
    preserveHandEdits: false, unknownField: 'strip me too',
  });
  assert.equal(replaced.preserveHandEdits, true);
  assert.equal(replaced.keyframes.length, 80);
  assert.equal(Object.hasOwn(replaced, 'unknownField'), false);
  assert.equal(PM.AgentHarness.cleanCommand({ type: 'run_shell', command: 'whoami' }), null);
});

test('the native prompt explicitly freezes the current harness vocabulary discrepancy', () => {
  const match = agentInstructions().match(/Supported Powermove command types are:\s*([^.]+)\./);
  assert.ok(match, 'native instructions must list the supported command vocabulary');
  const promptOperations = match[1].split(',').map(value => value.trim());
  assert.equal(promptOperations.length, 18);
  const harnessOperations = new Set(sceneOperations());
  const promptOnly = promptOperations.filter(operation => !harnessOperations.has(operation)).sort();
  const harnessOnly = [...harnessOperations].filter(operation => !promptOperations.includes(operation)).sort();

  /* Phase 3a will generate the prompt from Edit.operations; flip this assertion then. */
  assert.equal(harnessOperations.size, 15);
  assert.deepEqual(harnessOnly, []);
  assert.deepEqual(promptOnly, ['create_section', 'set_easing', 'update_section']);
});
