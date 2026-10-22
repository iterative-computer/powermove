#!/usr/bin/env node
/*
 * Document writer inventory for the native engine (docs/rust-engine.md,
 * section 6, "Making every write visible").
 *
 * Scans apps/desktop/src (tests, test helpers, generated public bundles, and
 * Finder duplicates excluded) with the TypeScript parser, and Svelte
 * components through the Svelte parser, for every site that writes the
 * document, and prints stable, sorted JSON: one entry per site with its
 * file:line, a class, the enclosing scope, and the code.
 *
 *   node scripts/engine-writers.mjs            print the inventory
 *   node scripts/engine-writers.mjs --write    update docs/engine/writers.json
 *   node scripts/engine-writers.mjs --check    exit 1 when the snapshot is stale
 *
 * The checked-in snapshot is compared by a unit test, so a new writer fails
 * until someone regenerates the snapshot and reviews its class.
 *
 * What counts as a write:
 * - calls into the history and edit entry points (`PM.hist.do`,
 *   `api.history.do`, `Edit.mutate`, history transactions and external
 *   entries, `replaceProject`), `PM.touch`, `api.anim.touch`, shader uniform
 *   synchronization, and every mention of `api.host.pm`;
 * - assignments, `++`/`--`, `delete`, mutating array methods, and
 *   `Object.assign`/`Object.defineProperty` whose target is `PM.proj` or an
 *   alias of it (a variable initialized from a `PM.proj` chain, a `for…of`
 *   variable over one, or the callback parameter of an array method on one),
 *   unless the write sits inside a callback passed to a history entry point,
 *   which already accounts for it;
 * - the named sites of section 6 (NAMED_SITES below). A named site that no
 *   longer matches anything is an error, so renames cannot hide a writer.
 *
 * Writes through objects the parser cannot connect to `PM.proj` (layers
 * fetched by id, values passed between modules) are outside a syntactic
 * scan; the development write barrier (section 6, item 1) catches those.
 */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const SOURCE_ROOT = path.join(REPO_ROOT, 'apps', 'desktop', 'src');
export const SNAPSHOT = path.join(REPO_ROOT, 'docs', 'engine', 'writers.json');

const desktopRequire = createRequire(path.join(REPO_ROOT, 'apps', 'desktop', 'package.json'));
const ts = desktopRequire('typescript');
let svelteParse;
async function svelteParser() {
  if (!svelteParse) {
    const compiler = await import(pathToFileURL(desktopRequire.resolve('svelte/compiler')).href);
    svelteParse = compiler.parse ?? compiler.default?.parse;
    if (typeof svelteParse !== 'function') throw new Error('svelte/compiler has no parse export.');
  }
  return svelteParse;
}

export const CLASSES = {
  'hist.do': 'PM.hist.do: one undoable step around a mutation callback',
  'history.do': 'api.history.do: the kernel history step',
  'edit.mutate': 'Edit.mutate / api.edit.mutate: a validated structural mutation callback',
  'history.transaction': 'hist.begin / history.begin: a raw transaction; writes until commit or cancel',
  'history.external': 'hist.external / history.external: an external undo entry around host-applied changes',
  'project-replace': 'replaceProject: the whole-document restore path (undo, load, rollback, remote sync)',
  touch: 'PM.touch: a write that only announces the document changed (the barrier diffs the open composition)',
  'touch.paths': 'PM.touch(paths): a write that announces exactly which paths changed',
  'doc.write': 'PM.doc.write or writeDocument (engine-doc/write.ts): a write-barrier writer scope around a mutation, with its paths',
  'doc.touched': 'PM.doc.touched or touchedDocument (engine-doc/write.ts): a write-barrier record of paths already written',
  'anim.touch': 'api.anim.touch: the kernel form of PM.touch',
  'shader-uniforms': 'shaderHooks.syncShaderUniforms: rewrites shader layer uniforms from source',
  'host.pm': 'api.host.pm: hands the raw PM registry (and PM.proj) to extensions',
  'project-swap': 'PM.proj = …: replaces the live document object outside history',
  revision: 'a document revision counter write',
  'asset-metadata': 'a write into PM.proj.assets outside history',
  'project-write': 'a write into PM.proj or an alias of it outside history',
  'render-queue-swap': 'the render queue swapping the live project for a queued job',
  'home-composition-match': "Home's Open matching the new composition to the first media source",
  'agent-media-folders': 'agent media import placing assets into folders',
  'extension-layer-refresh': 'extension layers refilled with parameters when a definition registers (through writeDocument)',
  'aim-preserving': 'aim-preserving rewrites of 3D camera and light channels after reparenting',
};

/**
 * The patch origin (contract 5, `PatchOrigin`) each class's writes reach the
 * mirror with. `structuralDiff`: the barrier finds them by diffing (a
 * `touch()` without paths, or the consistency check before saves and exports).
 */
export const ORIGINS = {
  'hist.do': 'edit', 'history.do': 'edit', 'edit.mutate': 'edit', 'history.transaction': 'edit', 'history.external': 'edit',
  'project-replace': 'load', touch: 'structuralDiff', 'touch.paths': 'edit', 'doc.write': 'edit', 'doc.touched': 'edit',
  'anim.touch': 'structuralDiff', 'shader-uniforms': 'metadata', 'host.pm': 'structuralDiff', 'project-swap': 'load',
  revision: 'metadata', 'asset-metadata': 'metadata', 'project-write': 'structuralDiff', 'render-queue-swap': 'load',
  'home-composition-match': 'edit', 'agent-media-folders': 'edit',
  'extension-layer-refresh': 'metadata', 'aim-preserving': 'edit',
};

/*
 * Section 6 names these sites explicitly. `file` is relative to
 * apps/desktop/src; `scope` matches the enclosing scope path; `from` limits
 * the override to writes of one generic class; `code` filters by source text.
 * `params` treats the scope's parameters (and values derived from them) as
 * document objects, for functions that receive layers or channels.
 */
export const NAMED_SITES = [
  { class: 'render-queue-swap', file: 'renderer/src/legacy/core/render-queue.ts', from: ['project-swap'] },
  // The match runs inside writeDocument (a barrier writer): the call is the site.
  { class: 'home-composition-match', file: 'renderer/src/legacy/app.ts', scope: /(^| > )importFiles$/, from: ['doc.write'], code: /^writeDocument\(targetProject === PM\.proj/ },
  { class: 'agent-media-folders', file: 'renderer/src/legacy/assistant/media-management.ts', scope: /(^| > )importMedia$/, code: /\.folder\s*=/ },
  // The refresh runs inside writeDocument (a barrier writer): the call is the site.
  { class: 'extension-layer-refresh', file: 'renderer/src/kernel/install.ts', scope: /kernel\.layerTypes\.onChange\(callback\)$/, from: ['doc.write'] },
  { class: 'aim-preserving', file: 'renderer/src/legacy/core/parenting.ts', scope: /(^| > )preserveWorldAim$/, params: true },
];

const SOURCE_EXTENSIONS = new Set(['.ts', '.mts', '.js', '.mjs', '.svelte']);

/** True for files the inventory covers; `relative` uses forward slashes. */
export function isScannedFile(relative) {
  if (!SOURCE_EXTENSIONS.has(path.extname(relative))) return false;
  if (relative.endsWith('.d.ts')) return false;
  if (/(^|\/)(__tests__|__fixtures__|__mocks__|test-setup|node_modules)\//.test(relative)) return false;
  if (/^renderer\/public\//.test(relative)) return false;
  if (/\.(test|spec)\.[cm]?[jt]s$/.test(relative) || /\.test-helper\.[cm]?[jt]s$/.test(relative)) return false;
  // Finder duplicates such as "patch 2.ts" or a "folder 2" directory.
  if (/(^|\/)[^/]* \d+(\.[^/]*)?(\/|$)/.test(relative)) return false;
  return true;
}

function listFiles(directory, prefix = '') {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...listFiles(path.join(directory, entry.name), relative));
    else if (entry.isFile() && isScannedFile(relative)) files.push(relative);
  }
  return files;
}

/* ── chains and roots ─────────────────────────────────────────────────── */

function unwrap(node) {
  for (;;) {
    if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)
      || ts.isTypeAssertionExpression?.(node) || ts.isSatisfiesExpression?.(node)) node = node.expression;
    else return node;
  }
}

/**
 * The access chain of an expression as segments, e.g. `window.PM?.proj.assets[id]`
 * → ['PM', 'proj', 'assets', '[]']; null when the chain does not start at an
 * identifier or `this`.
 */
function chain(node) {
  node = unwrap(node);
  if (ts.isIdentifier(node)) return [node.text];
  if (node.kind === ts.SyntaxKind.ThisKeyword) return ['this'];
  if (ts.isPropertyAccessExpression(node)) {
    const base = chain(node.expression);
    return base && [...base, node.name.text];
  }
  if (ts.isElementAccessExpression(node)) {
    const base = chain(node.expression);
    return base && [...base, '[]'];
  }
  if (ts.isCallExpression(node)) {
    const base = chain(node.expression);
    return base && [...base, '()'];
  }
  return null;
}

function normalizedChain(node) {
  const segments = chain(node);
  if (!segments) return null;
  while (segments.length > 1 && (segments[0] === 'window' || segments[0] === 'globalThis')) segments.shift();
  return segments;
}

const dotted = segments => segments.join('.');

/** Segments after the PM root of a `PM.proj` chain (also `x.host.pm.proj`), else null. */
function projectPath(segments) {
  if (!segments) return null;
  if (segments[0] === 'PM' && segments[1] === 'proj') return segments.slice(2);
  const host = segments.indexOf('host');
  if (host >= 0 && segments[host + 1] === 'pm' && segments[host + 2] === 'proj') return segments.slice(host + 3);
  return null;
}

/* ── per-file scan ────────────────────────────────────────────────────── */

const ASSIGNMENT_OPERATORS = new Set([
  ts.SyntaxKind.EqualsToken, ts.SyntaxKind.PlusEqualsToken, ts.SyntaxKind.MinusEqualsToken,
  ts.SyntaxKind.AsteriskEqualsToken, ts.SyntaxKind.AsteriskAsteriskEqualsToken, ts.SyntaxKind.SlashEqualsToken,
  ts.SyntaxKind.PercentEqualsToken, ts.SyntaxKind.LessThanLessThanEqualsToken, ts.SyntaxKind.GreaterThanGreaterThanEqualsToken,
  ts.SyntaxKind.GreaterThanGreaterThanGreaterThanEqualsToken, ts.SyntaxKind.AmpersandEqualsToken, ts.SyntaxKind.BarEqualsToken,
  ts.SyntaxKind.CaretEqualsToken, ts.SyntaxKind.AmpersandAmpersandEqualsToken, ts.SyntaxKind.BarBarEqualsToken,
  ts.SyntaxKind.QuestionQuestionEqualsToken,
]);
const MUTATING_METHODS = new Set(['push', 'pop', 'shift', 'unshift', 'splice', 'sort', 'reverse', 'fill', 'copyWithin']);
const CALLBACK_METHODS = new Set(['forEach', 'map', 'filter', 'some', 'every', 'find', 'findLast', 'flatMap', 'reduce']);
/** Values these produce are copies, never live document objects. */
const COPYING_CALLS = /^(JSON\.(parse|stringify)|structuredClone|clone|cloneValue|String|Number|Boolean)$/;

/** Classify a call by its callee chain; null when it is not a writer entry point. */
function callClass(segments) {
  if (!segments) return null;
  const name = dotted(segments);
  if (/(^|\.)hist\.do$/.test(name)) return 'hist.do';
  if (/(^|\.)history\.do$/.test(name)) return 'history.do';
  if (/(^|\.)([A-Za-z]*Edit|edit)\.mutate$/.test(name)) return 'edit.mutate';
  if (/(^|\.)(hist|history)\.begin$/.test(name)) return 'history.transaction';
  if (/(^|\.)(hist|history)\.external$/.test(name)) return 'history.external';
  if (/(^|\.)replaceProject$/.test(name)) return 'project-replace';
  if (name === 'PM.touch' || /\.host\.pm\.touch$/.test(name)) return 'touch';
  if (/(^|\.)doc\.write$/.test(name) || name === 'writeDocument') return 'doc.write';
  if (/(^|\.)doc\.touched$/.test(name) || name === 'touchedDocument') return 'doc.touched';
  if (/(^|\.)anim\.touch$/.test(name)) return 'anim.touch';
  if (/(^|\.)syncShaderUniforms$/.test(name)) return 'shader-uniforms';
  return null;
}
/** Entry points whose callback arguments run inside history. */
const HISTORY_CALLBACKS = new Set(['hist.do', 'history.do', 'edit.mutate', 'doc.write']);

function functionName(node) {
  if (ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node) || ts.isGetAccessorDeclaration(node) || ts.isSetAccessorDeclaration(node)) {
    const own = node.name ? node.name.getText() : '(anonymous)';
    const owner = node.parent && (ts.isClassDeclaration(node.parent) || ts.isClassExpression(node.parent)) && node.parent.name ? `${node.parent.name.text}.` : '';
    return owner + own;
  }
  if (ts.isConstructorDeclaration(node)) return `${node.parent?.name?.text ?? '(class)'}.constructor`;
  const parent = node.parent;
  if (!parent) return '(anonymous)';
  if (ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) return parent.name.text;
  if ((ts.isPropertyAssignment(parent) || ts.isPropertyDeclaration(parent)) && parent.name) return parent.name.getText();
  if (ts.isBinaryExpression(parent) && parent.right === node) {
    const target = normalizedChain(parent.left);
    if (target) return dotted(target);
  }
  if (ts.isCallExpression(parent)) {
    const callee = normalizedChain(parent.expression);
    return `${callee ? dotted(callee) : '(call)'}(callback)`;
  }
  return '(anonymous)';
}

const isFunctionLike = node => ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node) || ts.isArrowFunction(node)
  || ts.isMethodDeclaration(node) || ts.isConstructorDeclaration(node) || ts.isGetAccessorDeclaration(node) || ts.isSetAccessorDeclaration(node);

function bindingNames(name, out = []) {
  if (ts.isIdentifier(name)) out.push(name.text);
  else if (ts.isObjectBindingPattern(name) || ts.isArrayBindingPattern(name)) {
    for (const element of name.elements) if (!ts.isOmittedExpression(element)) bindingNames(element.name, out);
  }
  return out;
}

function snippet(node, source) {
  const text = node.getText(source).replace(/\s+/g, ' ').trim();
  return text.length > 140 ? `${text.slice(0, 137)}...` : text;
}

/**
 * Local helpers that hand one of their parameters straight to a history
 * entry point, e.g. `const transact = (label, fn) => PM.hist.do(label, fn)`.
 * Returns name → { class, index } for the forwarded parameter.
 */
function historyWrappers(source) {
  const wrappers = new Map();
  const visit = node => {
    let name = null, fn = null;
    if (ts.isFunctionDeclaration(node) && node.name && node.body) { name = node.name.text; fn = node; }
    else if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer
      && (ts.isArrowFunction(unwrap(node.initializer)) || ts.isFunctionExpression(unwrap(node.initializer)))) { name = node.name.text; fn = unwrap(node.initializer); }
    if (fn) {
      const parameters = fn.parameters.map(parameter => (ts.isIdentifier(parameter.name) ? parameter.name.text : null));
      const scan = inner => {
        if (ts.isCallExpression(inner)) {
          const cls = callClass(normalizedChain(inner.expression));
          if (HISTORY_CALLBACKS.has(cls)) {
            // do(label, fn) and mutate(label, fn, meta) take the callback second:
            // either the parameter itself, or a callback that calls it.
            const argument = inner.arguments[1] && unwrap(inner.arguments[1]);
            let index = argument && ts.isIdentifier(argument) ? parameters.indexOf(argument.text) : -1;
            if (index < 0 && argument && (ts.isArrowFunction(argument) || ts.isFunctionExpression(argument))) {
              const calls = body => {
                if (index >= 0) return;
                if (ts.isCallExpression(body) && ts.isIdentifier(unwrap(body.expression))) index = parameters.indexOf(unwrap(body.expression).text);
                ts.forEachChild(body, calls);
              };
              calls(argument.body);
            }
            if (index >= 0 && !wrappers.has(name)) wrappers.set(name, { class: cls, index });
          }
        }
        ts.forEachChild(inner, scan);
      };
      scan(fn.body);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return wrappers;
}

/**
 * Scan one parsed source. `file` is relative to apps/desktop/src. Returns raw
 * entries before named-site overrides: { file, line, class, scope, code, generic }.
 */
function scanSourceFile(source, file, shared = { wrappers: new Map(), taint: new Map() }) {
  const entries = [];
  const named = NAMED_SITES.filter(site => site.file === file);
  // A Svelte component's script and template chunks share wrappers and aliases.
  const wrappers = shared.wrappers;
  for (const [name, wrapper] of historyWrappers(source)) if (!wrappers.has(name)) wrappers.set(name, wrapper);
  /** The history class of a call, counting local wrappers; with the callback argument index. */
  const historyCall = call => {
    const cls = callClass(normalizedChain(call.expression));
    if (HISTORY_CALLBACKS.has(cls)) return { class: cls, index: null };
    const callee = unwrap(call.expression);
    const wrapper = ts.isIdentifier(callee) ? wrappers.get(callee.text) : null;
    return wrapper ? { class: wrapper.class, index: wrapper.index, wrapper: true } : null;
  };
  // Scope frames: { name, path, taint: Map<identifier, string[] project path>, named }.
  const frames = [{ name: '(module)', path: '(module)', taint: shared.taint, site: null }];
  let historyDepth = 0;

  const scopePath = () => frames.at(-1).path;
  const lookup = name => {
    for (let index = frames.length - 1; index >= 0; index--) if (frames[index].taint.has(name)) return frames[index].taint.get(name);
    return null;
  };
  const activeSite = () => {
    for (let index = frames.length - 1; index >= 0; index--) if (frames[index].site) return frames[index].site;
    return null;
  };
  /** Project path of an expression that refers to the document, else null. */
  const rootOf = expression => {
    const segments = normalizedChain(expression);
    if (!segments) return null;
    const direct = projectPath(segments);
    if (direct) return direct;
    const alias = lookup(segments[0]);
    return alias ? [...alias, ...segments.slice(1)] : null;
  };
  /** In derived scopes, any expression that mentions a document reference. */
  const mentionsDocument = expression => {
    let found = false;
    const visit = node => {
      if (found) return;
      if (ts.isCallExpression(node)) {
        const callee = normalizedChain(node.expression);
        if (callee && COPYING_CALLS.test(dotted(callee))) return;
      }
      if ((ts.isIdentifier(node) || ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) && rootOf(node)) { found = true; return; }
      ts.forEachChild(node, visit);
    };
    visit(expression);
    return found;
  };
  const derivedPath = expression => {
    const direct = rootOf(expression);
    if (direct) return direct;
    return activeSite()?.derived || activeSite()?.params ? (mentionsDocument(expression) ? ['?'] : null) : null;
  };
  const taint = (name, projectSegments) => { if (projectSegments) frames.at(-1).taint.set(name, projectSegments); };

  const record = (node, cls, generic) => {
    const { line } = source.getLineAndCharacterOfPosition(node.getStart(source));
    entries.push({ file, line: line + 1, class: cls, scope: scopePath(), code: snippet(node, source), generic });
  };

  const writeClass = (projectSegments, replaces) => {
    // Only `PM.proj = …` swaps the document; Object.assign(PM.proj, …) edits it.
    if (!projectSegments.length) return replaces ? 'project-swap' : 'project-write';
    const properties = projectSegments.filter(segment => segment !== '[]' && segment !== '()' && segment !== '?');
    if (properties.at(-1) === 'revision') return 'revision';
    if (projectSegments[0] === 'assets') return 'asset-metadata';
    return 'project-write';
  };
  const recordWrite = (node, target, replaces = false) => {
    if (historyDepth > 0) return;
    // Rebinding a local variable, even an alias, never writes the document.
    if (ts.isIdentifier(unwrap(target))) return;
    const projectSegments = rootOf(target);
    if (!projectSegments) return;
    record(node, writeClass(projectSegments, replaces), true);
  };

  const visit = node => {
    // Function scopes.
    if (isFunctionLike(node)) {
      const name = functionName(node);
      const parentPath = scopePath();
      const framePath = parentPath === '(module)' ? name : `${parentPath} > ${name}`;
      const site = named.find(item => item.scope && (item.params || item.derived) && item.scope.test(framePath)) || null;
      const frame = { name, path: framePath, taint: new Map(), site };
      frames.push(frame);
      if (site?.params) for (const parameter of node.parameters) for (const id of bindingNames(parameter.name)) frame.taint.set(id, ['?']);
      // Callback parameters of array methods on document chains are aliases.
      const parent = node.parent;
      if (parent && ts.isCallExpression(parent) && parent.arguments[0] === node && ts.isPropertyAccessExpression(unwrap(parent.expression))) {
        const access = unwrap(parent.expression);
        if (CALLBACK_METHODS.has(access.name.text)) {
          const collection = derivedPath(access.expression);
          const index = access.name.text === 'reduce' ? 1 : 0;
          const parameter = node.parameters[index];
          if (collection && parameter) for (const id of bindingNames(parameter.name)) frame.taint.set(id, [...collection, '[]']);
        }
      }
      const viaHistory = parent && ts.isCallExpression(parent) && parent.arguments.includes(node) ? historyCall(parent) : null;
      const historyCallback = !!viaHistory && (viaHistory.index === null || parent.arguments.indexOf(node) === viaHistory.index);
      if (historyCallback) historyDepth++;
      ts.forEachChild(node, visit);
      if (historyCallback) historyDepth--;
      frames.pop();
      return;
    }

    // Aliases.
    if (ts.isVariableDeclaration(node) && node.initializer) {
      ts.forEachChild(node, visit);
      const initializer = unwrap(node.initializer);
      const candidates = ts.isBinaryExpression(initializer)
        && [ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.AmpersandAmpersandToken].includes(initializer.operatorToken.kind)
        ? [initializer.left, initializer.right]
        : ts.isConditionalExpression(initializer) ? [initializer.whenTrue, initializer.whenFalse] : [initializer];
      const projectSegments = candidates.map(derivedPath).find(Boolean);
      if (projectSegments) {
        if (ts.isIdentifier(node.name)) taint(node.name.text, projectSegments);
        else for (const id of bindingNames(node.name)) taint(id, [...projectSegments, '?']);
      }
      return;
    }
    if (ts.isForOfStatement(node) || ts.isForInStatement(node)) {
      const collection = ts.isForOfStatement(node) ? derivedPath(node.expression) : null;
      visit(node.expression);
      if (collection && ts.isVariableDeclarationList(node.initializer)) {
        for (const declaration of node.initializer.declarations) for (const id of bindingNames(declaration.name)) taint(id, [...collection, '[]']);
      }
      visit(node.initializer);
      visit(node.statement);
      return;
    }

    // Writes.
    if (ts.isBinaryExpression(node) && ASSIGNMENT_OPERATORS.has(node.operatorToken.kind)) {
      recordWrite(node, node.left, node.operatorToken.kind === ts.SyntaxKind.EqualsToken);
    } else if ((ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node))
      && (node.operator === ts.SyntaxKind.PlusPlusToken || node.operator === ts.SyntaxKind.MinusMinusToken)) {
      recordWrite(node, node.operand);
    } else if (ts.isDeleteExpression(node)) {
      recordWrite(node, node.expression);
    } else if (ts.isCallExpression(node)) {
      const callee = normalizedChain(node.expression);
      let cls = callClass(callee);
      // `PM.touch(paths)` names what changed; a bare touch asks for a diff.
      if (cls === 'touch' && node.arguments.length) cls = 'touch.paths';
      if (cls) record(node, cls, false);
      else {
        const wrapped = historyCall(node);
        if (wrapped?.wrapper) record(node, wrapped.class, false);
      }
      const access = unwrap(node.expression);
      if (ts.isPropertyAccessExpression(access) && MUTATING_METHODS.has(access.name.text)) recordWrite(node, access.expression);
      if (callee && (dotted(callee) === 'Object.assign' || dotted(callee) === 'Object.defineProperty') && node.arguments[0]) recordWrite(node, node.arguments[0]);
    } else if ((ts.isMethodDeclaration(node) || ts.isPropertyAssignment(node)) && node.name?.getText() === 'syncShaderUniforms') {
      record(node.name, 'shader-uniforms', false);
    }

    // api.host.pm: every mention of the property, and where the API defines it.
    if (ts.isPropertyAccessExpression(node) && node.name.text === 'pm') {
      const segments = normalizedChain(node);
      if (segments && segments.at(-2) === 'host') record(node, 'host.pm', false);
    }
    if (ts.isPropertyAssignment(node) && node.name.getText() === 'pm' && ts.isObjectLiteralExpression(node.parent)
      && ts.isPropertyAssignment(node.parent.parent) && node.parent.parent.name.getText() === 'host') {
      record(node, 'host.pm', false);
    }

    ts.forEachChild(node, visit);
  };
  visit(source);
  return entries;
}

function scriptKind(file) {
  const extension = path.extname(file);
  return extension === '.js' || extension === '.mjs' ? ts.ScriptKind.JS : ts.ScriptKind.TS;
}

/** Blank everything before `start` except newlines, so parsed lines match the file. */
const padded = (text, start, body) => text.slice(0, start).replace(/[^\n]/g, ' ') + body;

const SVELTE_NODES = new Set([
  'Root', 'Fragment', 'Text', 'Comment', 'ExpressionTag', 'HtmlTag', 'ConstTag', 'DebugTag', 'RenderTag', 'AttachTag',
  'RegularElement', 'Component', 'SvelteComponent', 'SvelteElement', 'SvelteSelf', 'SvelteFragment', 'SvelteHead',
  'SvelteBody', 'SvelteWindow', 'SvelteDocument', 'SvelteOptions', 'SvelteBoundary', 'TitleElement', 'SlotElement',
  'Attribute', 'SpreadAttribute', 'AnimateDirective', 'BindDirective', 'ClassDirective', 'LetDirective', 'OnDirective',
  'StyleDirective', 'TransitionDirective', 'UseDirective', 'IfBlock', 'EachBlock', 'AwaitBlock', 'KeyBlock', 'SnippetBlock',
]);

/** Source ranges of every JavaScript expression or declaration in a Svelte template. */
function templateRanges(fragment) {
  const ranges = [];
  const walk = value => {
    if (!value || typeof value !== 'object') return;
    if (Array.isArray(value)) { value.forEach(walk); return; }
    if (typeof value.type === 'string' && !SVELTE_NODES.has(value.type)) {
      if (typeof value.start === 'number' && typeof value.end === 'number') ranges.push(value);
      return;
    }
    for (const [key, child] of Object.entries(value)) if (key !== 'metadata' && key !== 'parent') walk(child);
  };
  walk(fragment);
  return ranges;
}

/** Inventory entries for one file's text; `file` is relative to apps/desktop/src. */
export async function scanFile(file, text) {
  if (path.extname(file) !== '.svelte') {
    return scanSourceFile(ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, scriptKind(file)), file);
  }
  const parse = await svelteParser();
  const root = parse(text, { modern: true });
  const entries = [];
  const shared = { wrappers: new Map(), taint: new Map() };
  const scripts = [root.module, root.instance].filter(script => script?.content)
    .map(script => ts.createSourceFile(file, padded(text, script.content.start, text.slice(script.content.start, script.content.end)), ts.ScriptTarget.Latest, true, ts.ScriptKind.TS));
  const template = templateRanges(root.fragment).sort((a, b) => a.start - b.start).map(node => {
    const body = text.slice(node.start, node.end);
    const statement = /Declaration$|Statement$/.test(node.type) ? body : `(${body});`;
    return ts.createSourceFile(file, padded(text, node.start, statement), ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  });
  // Wrappers first, so a template call to a script helper counts as history.
  for (const source of [...scripts, ...template]) for (const [name, wrapper] of historyWrappers(source)) if (!shared.wrappers.has(name)) shared.wrappers.set(name, wrapper);
  for (const source of scripts) entries.push(...scanSourceFile(source, file, shared));
  for (const source of template) {
    entries.push(...scanSourceFile(source, file, shared).map(entry => ({ ...entry, scope: entry.scope === '(module)' ? '(template)' : `(template) > ${entry.scope}` })));
  }
  return entries;
}

function applyNamedSites(entries) {
  const matched = new Set();
  const result = entries.map(entry => {
    const site = NAMED_SITES.find(item => item.file === entry.file
      && (!item.scope || item.scope.test(entry.scope))
      && (!item.from || item.from.includes(entry.class))
      && (!item.code || item.code.test(entry.code))
      && (item.from || entry.generic));
    if (!site) return entry;
    matched.add(site);
    return { ...entry, class: site.class };
  });
  const missing = NAMED_SITES.filter(site => !matched.has(site));
  if (missing.length) {
    throw new Error(`Named writer sites no longer match any code: ${missing.map(site => `${site.class} (${site.file})`).join(', ')}. `
      + 'Update NAMED_SITES in scripts/engine-writers.mjs.');
  }
  return result;
}

const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** The full inventory as a plain object; deterministic for a given tree. */
export async function collectWriters(root = SOURCE_ROOT) {
  const files = listFiles(root).sort(compare);
  const raw = [];
  for (const file of files) raw.push(...await scanFile(file, readFileSync(path.join(root, file), 'utf8')));
  const prefix = path.relative(REPO_ROOT, root).split(path.sep).join('/');
  const seen = new Set();
  const writers = applyNamedSites(raw)
    .map(entry => ({ site: `${prefix}/${entry.file}:${entry.line}`, file: entry.file, line: entry.line, class: entry.class, scope: entry.scope, code: entry.code }))
    .sort((a, b) => compare(a.file, b.file) || a.line - b.line || compare(a.class, b.class) || compare(a.code, b.code))
    .filter(entry => {
      const key = `${entry.site}\n${entry.class}\n${entry.code}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map(({ site, class: cls, scope, code }) => ({ site, class: cls, scope, code }));
  const counts = Object.fromEntries(Object.keys(CLASSES).map(cls => [cls, writers.filter(entry => entry.class === cls).length]));
  return {
    description: 'Document writer inventory (docs/rust-engine.md section 6). Generated by scripts/engine-writers.mjs; regenerate with --write and review every new entry.',
    root: prefix,
    classes: CLASSES,
    origins: ORIGINS,
    counts,
    writers,
  };
}

export const serialize = inventory => `${JSON.stringify(inventory, null, 2)}\n`;

async function main(argv) {
  const output = serialize(await collectWriters());
  if (argv.includes('--write')) {
    writeFileSync(SNAPSHOT, output);
    process.stdout.write(`Wrote ${path.relative(REPO_ROOT, SNAPSHOT)}\n`);
    return 0;
  }
  if (argv.includes('--check')) {
    let current = '';
    try { current = readFileSync(SNAPSHOT, 'utf8'); } catch { /* missing snapshot is stale */ }
    if (current === output) return 0;
    process.stderr.write(`${path.relative(REPO_ROOT, SNAPSHOT)} is stale. Run node scripts/engine-writers.mjs --write and review the new writers.\n`);
    return 1;
  }
  process.stdout.write(output);
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then(code => { process.exitCode = code; }, error => { console.error(error); process.exitCode = 1; });
}
