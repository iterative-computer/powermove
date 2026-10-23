/*
 * Deprecated compatibility API use in extension source (plan section 26,
 * amendment 23).
 *
 * Every compatibility member keeps working at apiVersion 1 to 3. This module
 * only finds where source uses one, so the tools that read, compile, validate
 * or publish an extension can return a structured warning naming the file,
 * the line, and the replacement. It never blocks, rewrites, or disables
 * anything: callers attach the warnings to their results.
 *
 * The analysis is lexical and best-effort. It follows the API object through
 * the `activate` parameter, any identifier annotated `PowermoveAPI`, the
 * conventional name `api`, plain aliases (`const r = api.render`),
 * destructuring (`const { snapshot } = api.render`), computed keys with a
 * literal (`api.render['snapshot']`), and call results
 * (`const a = api.media.assets.get(id); a.el`). A computed key that is not a
 * literal on an object with deprecated members is reported as a possible use
 * (`access: 'dynamic'`). It does not follow values through function
 * parameters other than `activate`'s, object properties, or module imports.
 */

/** One deprecated compatibility member and its replacement. */
export interface DeprecatedMember {
  /** Stable id: the member's path on the API object (`render.snapshot`). */
  id: string;
  /** How source reaches it, from the API object; `()` is a call result. */
  path: readonly string[];
  /** For a service: the service name passed to `services.get/register`. */
  service?: string;
  /** The replacement to use in new code, as source. */
  replacement: string;
  /** One sentence on what the replacement does differently. */
  note: string;
}

/** The single source of truth for deprecated members (EXTENSIONS.md mirrors it, and a test keeps them equal). */
export const DEPRECATED_MEMBERS: readonly DeprecatedMember[] = [
  {
    id: 'render.snapshot',
    path: ['render', 'snapshot'],
    replacement: 'await api.render.snapshotAsync(time, maxWidth)',
    note: 'Resolves to the same JPEG data URL without blocking the editor while the engine draws the frame.'
  },
  {
    id: 'render.renderFrameTo',
    path: ['render', 'renderFrameTo'],
    replacement: 'await api.render.renderFrameToAsync(time, width, height, options)',
    note: 'Resolves to the same opaque canvas without blocking the editor.'
  },
  {
    id: 'render.raster',
    path: ['render', 'raster'],
    replacement: 'await api.render.rasterAsync(layer, scale, time, crop)',
    note: 'Resolves to the same raster surface (pixels, anchor, selection, fontOffset) without blocking the editor.'
  },
  {
    id: 'render.gl.context',
    path: ['render', 'gl', 'context'],
    replacement: "api.layers.register({ renderer: { kind: 'fragment', fragment } }) to draw, await api.render.renderFrameToAsync(time, width, height) to read the frame",
    note: 'A registered GLSL layer is drawn by the engine in preview and export; the compatibility context is a separate offscreen context that reads the frame on screen back and shows its drawing in the viewer until the next frame and in WebM exports after each frame:rendered handler; MP4, ProRes, PNG and still captures read the engine frame without that drawing.'
  },
  {
    id: 'services.raster',
    path: ['services'],
    service: 'raster',
    replacement: 'api.layers.register or api.effects.register (GLSL) to change pixels, await api.render.rasterAsync(layer) to read a layer',
    note: 'Engine-drawn layers and effects need no synchronous round trip per layer and frame.'
  },
  {
    id: 'media.assets.get().el',
    path: ['media', 'assets', 'get', '()', 'el'],
    replacement: 'await api.media.assets.source(id) and an element of your own',
    note: 'Resolves to a browser-playable URL for the asset; your element owns its playback state.'
  },
  {
    id: 'media.assets.get().url',
    path: ['media', 'assets', 'get', '()', 'url'],
    replacement: 'await api.media.assets.source(id)',
    note: 'Resolves once a browser-playable rendition exists, instead of reading a field that may not be set yet.'
  },
  {
    id: 'media.assets.get().audioBlob',
    path: ['media', 'assets', 'get', '()', 'audioBlob'],
    replacement: 'await api.media.assets.audio(id)',
    note: 'Resolves to the asset\'s audio as a Blob once it is available.'
  },
  {
    id: 'media.assets.get().object3d',
    path: ['media', 'assets', 'get', '()', 'object3d'],
    replacement: 'api.scene3d.describe(layerId)',
    note: 'Describes the model\'s objects, materials and lights as data instead of a live three.js object.'
  },
  {
    id: 'host.pm',
    path: ['host', 'pm'],
    replacement: 'the typed namespaces (api.project, api.anim, api.model, api.transport, …)',
    note: 'The legacy registry has no stable shape; every capability built-ins use is on a typed namespace.'
  },
  {
    id: 'host.state',
    path: ['host', 'state'],
    replacement: 'api.project.latest(), api.project.selection(), api.project.time() and api.project.playing() (reactive in Svelte)',
    note: 'The typed reads are reactive inside Svelte components and stable across versions.'
  }
];

export type DeprecationAccess = 'static' | 'alias' | 'destructured' | 'computed' | 'dynamic';

/** A deprecated member used in one place of one file. */
export interface DeprecationWarning {
  /** Path relative to the extension's folder. */
  file: string;
  line: number;
  column: number;
  /** `DeprecatedMember.id`. */
  member: string;
  replacement: string;
  /** How the use was found; `dynamic` means it may or may not be one. */
  access: DeprecationAccess;
  /** One self-contained sentence for the agent or the developer. */
  message: string;
}

export interface SourceFile { path: string; text: string }

const SOURCE = /\.(?:[cm]?[jt]sx?|svelte)$/i;
const SKIP = /(?:^|\/)(?:node_modules|\.git)\/|\.d\.ts$|\.(?:test|spec)\.[cm]?[jt]sx?$/i;
/** Files larger than this are not analyzed (generated bundles). */
const MAX_FILE_CHARS = 2 * 1024 * 1024;
/** At most this many warnings per extension: the first ones locate the pattern. */
export const MAX_WARNINGS = 200;

/** The warnings for an extension's files, sorted by file and line. Never throws. */
export function analyzeExtensionSource(files: readonly SourceFile[]): DeprecationWarning[] {
  const out: DeprecationWarning[] = [];
  const seen = new Set<string>();
  for (const file of files) {
    if (!SOURCE.test(file.path) || SKIP.test(file.path) || file.text.length > MAX_FILE_CHARS) continue;
    let found: DeprecationWarning[];
    try {
      const code = /\.svelte$/i.test(file.path) ? svelteCode(file.text) : file.text;
      found = analyzeCode(file.path, code);
    } catch {
      continue;
    }
    for (const warning of found) {
      const key = `${warning.file}:${warning.line}:${warning.member}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(warning);
    }
  }
  out.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column || a.member.localeCompare(b.member));
  return out.slice(0, MAX_WARNINGS);
}

/** A warning about one extension of a staged change set, publish or compile. */
export interface ExtensionDeprecationWarning extends DeprecationWarning {
  /** The extension id. */
  extension: string;
}

/** `analyzeExtensionSource` for one extension, tagged with its id. */
export function analyzeExtension(id: string, files: readonly SourceFile[]): ExtensionDeprecationWarning[] {
  return analyzeExtensionSource(files).map((warning) => ({ extension: id, ...warning }));
}

/** One line per warning, for text results. */
export function formatDeprecationWarning(warning: DeprecationWarning): string {
  return `${warning.file}:${warning.line}: ${warning.message}`;
}

/** What agents are told to do with these warnings (harness, MCP, runners). */
export const DEPRECATION_GUIDANCE = 'Deprecated compatibility APIs keep working fully at apiVersion 1 to 3: never use them in new code. When an extension you read, compile or publish uses one, tell the user which file and line and what replaces it, and suggest the migration. Change the code only when the user agrees; never rewrite it silently, and never raise apiVersion for it.';

function message(member: DeprecatedMember, access: DeprecationAccess): string {
  const name = member.service ? `the '${member.service}' service override (api.services)` : `api.${member.id}`;
  const lead = access === 'dynamic'
    ? `This computed access may reach ${name}, a deprecated compatibility API.`
    : `${name} is a deprecated compatibility API.`;
  return `${lead} It keeps working at apiVersion 1 to 3; new code should use ${member.replacement}. ${member.note} Suggest the migration to the user and change it only if they agree.`;
}

/* ── Svelte: keep scripts and template expressions, blank the rest ── */

function svelteCode(text: string): string {
  const out = text.split('');
  const blank = (from: number, to: number) => {
    for (let i = from; i < to; i++) if (out[i] !== '\n') out[i] = ' ';
  };
  const script = /<script\b[^>]*>([\s\S]*?)<\/script\s*>/gi;
  const kept: Array<[number, number]> = [];
  for (const match of text.matchAll(script)) {
    const start = match.index! + match[0].indexOf(match[1]!);
    kept.push([start, start + match[1]!.length]);
  }
  let cursor = 0;
  for (const [start, end] of kept) {
    blankMarkup(text, cursor, start, blank);
    cursor = end;
  }
  blankMarkup(text, cursor, text.length, blank);
  // Script tags themselves.
  for (const match of text.matchAll(/<\/?script\b[^>]*>/gi)) blank(match.index!, match.index! + match[0].length);
  return out.join('');
}

/** Blanks markup between `from` and `to`, keeping `{…}` expressions (balanced, string-aware). */
function blankMarkup(text: string, from: number, to: number, blank: (a: number, b: number) => void): void {
  let i = from;
  let markupStart = from;
  while (i < to) {
    if (text[i] === '<' && text.startsWith('<!--', i)) {
      const end = text.indexOf('-->', i + 4);
      i = end < 0 || end + 3 > to ? to : end + 3;
      continue;
    }
    if (text[i] !== '{') { i++; continue; }
    blank(markupStart, i + 1);
    let depth = 1;
    let j = i + 1;
    while (j < to && depth > 0) {
      const c = text[j]!;
      if (c === '"' || c === "'" || c === '`') { j = skipString(text, j, to); continue; }
      if (c === '{') depth++;
      else if (c === '}') depth--;
      j++;
    }
    // Svelte block tags ({#if …}, {/if}, {:else}, {@html …}) keep their expression.
    const head = /^\{\s*[#/:@]\w*/.exec(text.slice(i, Math.min(j, i + 16)));
    if (head) blank(i, i + head[0].length);
    if (j > i) blank(j - 1, j);
    markupStart = j;
    i = j;
  }
  blank(markupStart, to);
}

function skipString(text: string, start: number, end: number): number {
  const quote = text[start];
  let i = start + 1;
  while (i < end && text[i] !== quote) {
    if (text[i] === '\\') i++;
    i++;
  }
  return i + 1;
}

/* ── tokens ── */

type Token = { kind: 'id' | 'str' | 'num' | 'punct' | 'tmpl'; value: string; line: number; column: number; index: number };

const PUNCT = ['?.', '...', '=>', '===', '!==', '==', '!=', '&&', '||', '??', '<=', '>=', '+=', '-=', '*=', '/=', '++', '--'];
const KEYWORDS_BEFORE_REGEX = new Set(['return', 'typeof', 'instanceof', 'in', 'of', 'new', 'delete', 'void', 'throw', 'case', 'do', 'else', 'yield', 'await']);

function tokenize(code: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  let line = 1;
  let lineStart = 0;
  const n = code.length;
  const push = (kind: Token['kind'], value: string, at: number, atLine: number, atLineStart: number) =>
    tokens.push({ kind, value, line: atLine, column: at - atLineStart + 1, index: tokens.length });
  const regexAllowed = (): boolean => {
    const prev = tokens[tokens.length - 1];
    if (!prev) return true;
    if (prev.kind === 'id') return KEYWORDS_BEFORE_REGEX.has(prev.value);
    if (prev.kind === 'punct') return ![')', ']', '}'].includes(prev.value);
    return false;
  };
  while (i < n) {
    const c = code[i]!;
    if (c === '\n') { i++; line++; lineStart = i; continue; }
    if (c === ' ' || c === '\t' || c === '\r' || c === '\f' || c === '\v' || c === ' ' || c === '﻿') { i++; continue; }
    if (c === '/' && code[i + 1] === '/') { while (i < n && code[i] !== '\n') i++; continue; }
    if (c === '/' && code[i + 1] === '*') {
      i += 2;
      while (i < n && !(code[i] === '*' && code[i + 1] === '/')) { if (code[i] === '\n') { line++; lineStart = i + 1; } i++; }
      i += 2;
      continue;
    }
    if (c === '"' || c === "'") {
      const start = i, startLine = line, startLineStart = lineStart;
      let value = '';
      i++;
      while (i < n && code[i] !== c && code[i] !== '\n') {
        if (code[i] === '\\') { value += code[i + 1] ?? ''; i += 2; continue; }
        value += code[i];
        i++;
      }
      i++;
      push('str', value, start, startLine, startLineStart);
      continue;
    }
    if (c === '`') {
      // A template literal: its text is one token; expressions inside are tokenized.
      const start = i, startLine = line, startLineStart = lineStart;
      let value = '';
      let plain = true;
      i++;
      while (i < n && code[i] !== '`') {
        if (code[i] === '\\') { value += code[i + 1] ?? ''; i += 2; continue; }
        if (code[i] === '$' && code[i + 1] === '{') {
          plain = false;
          let depth = 1;
          const exprStart = i + 2;
          let j = exprStart;
          while (j < n && depth > 0) {
            if (code[j] === '{') depth++;
            else if (code[j] === '}') depth--;
            else if (code[j] === '"' || code[j] === "'" || code[j] === '`') { j = skipString(code, j, n) - 1; }
            j++;
          }
          const inner = tokenize(code.slice(exprStart, j - 1));
          // Inner positions are relative to the expression: rebase them.
          const before = code.slice(0, exprStart);
          const baseLine = before.split('\n').length;
          const baseCol = exprStart - (before.lastIndexOf('\n') + 1);
          push('tmpl', plain ? value : '', start, startLine, startLineStart);
          for (const t of inner) tokens.push({ ...t, line: t.line + baseLine - 1, column: t.line === 1 ? t.column + baseCol : t.column, index: tokens.length });
          for (let k = i; k < j; k++) if (code[k] === '\n') { line++; lineStart = k + 1; }
          i = j;
          continue;
        }
        if (code[i] === '\n') { line++; lineStart = i + 1; }
        value += code[i];
        i++;
      }
      i++;
      if (plain) push('tmpl', value, start, startLine, startLineStart);
      continue;
    }
    if (c === '/' && regexAllowed()) {
      let j = i + 1;
      let inClass = false;
      while (j < n && code[j] !== '\n') {
        if (code[j] === '\\') { j += 2; continue; }
        if (code[j] === '[') inClass = true;
        else if (code[j] === ']') inClass = false;
        else if (code[j] === '/' && !inClass) break;
        j++;
      }
      if (code[j] === '/') {
        j++;
        while (j < n && /[a-z]/i.test(code[j]!)) j++;
        push('punct', 'regex', i, line, lineStart);
        i = j;
        continue;
      }
    }
    if (/[A-Za-z_$\u0080-￿]/.test(c)) {
      const start = i;
      while (i < n && /[\w$\u0080-￿]/.test(code[i]!)) i++;
      push('id', code.slice(start, i), start, line, lineStart);
      continue;
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(code[i + 1] ?? ''))) {
      const start = i;
      while (i < n && /[\w.]/.test(code[i]!)) i++;
      push('num', code.slice(start, i), start, line, lineStart);
      continue;
    }
    const three = code.slice(i, i + 3), two = code.slice(i, i + 2);
    const op = PUNCT.find((p) => p === three) ?? PUNCT.find((p) => p === two);
    // `?.` followed by a digit is a conditional with a decimal.
    if (op && !(op === '?.' && /[0-9]/.test(code[i + 2] ?? ''))) { push('punct', op, i, line, lineStart); i += op.length; continue; }
    push('punct', c, i, line, lineStart);
    i++;
  }
  return tokens;
}

/* ── analysis ── */

type Path = string[];

/** Deprecated members by the parent path their last step hangs off. */
const BY_PATH = new Map<string, DeprecatedMember>(DEPRECATED_MEMBERS.filter((m) => !m.service).map((m) => [m.path.join('.'), m]));
const SERVICE = DEPRECATED_MEMBERS.filter((m) => m.service);
/** Parents whose unknown computed members may be deprecated. */
const DYNAMIC_PARENTS = new Map<string, DeprecatedMember[]>();
for (const member of DEPRECATED_MEMBERS) {
  if (member.service || member.path.includes('()')) continue;
  const parent = member.path.slice(0, -1).join('.');
  if (!parent) continue;
  DYNAMIC_PARENTS.set(parent, [...(DYNAMIC_PARENTS.get(parent) ?? []), member]);
}

/** Paths worth following: a prefix of a deprecated member's path. */
function relevant(path: Path): boolean {
  const key = path.join('.');
  if (!key) return true;
  if (SERVICE.length && (key === 'services.get' || key === 'services.register')) return true;
  for (const member of DEPRECATED_MEMBERS) {
    const full = member.path.join('.');
    if (full === key || full.startsWith(key + '.')) return true;
  }
  return false;
}

function analyzeCode(file: string, code: string): DeprecationWarning[] {
  const tokens = tokenize(code);
  const warnings: DeprecationWarning[] = [];
  const aliases = new Map<string, Path>();
  /** `const NAME = 'raster'`: string constants used as service names. */
  const strings = new Map<string, string>();
  const warn = (member: DeprecatedMember, token: Token, access: DeprecationAccess) =>
    warnings.push({ file, line: token.line, column: token.column, member: member.id, replacement: member.replacement, access, message: message(member, access) });

  // Roots: `api`, identifiers annotated PowermoveAPI, `activate`'s parameter.
  aliases.set('api', []);
  for (let k = 0; k < tokens.length; k++) {
    const t = tokens[k]!;
    if (t.kind === 'id' && t.value === 'PowermoveAPI' && tokens[k - 1]?.value === ':' && tokens[k - 2]?.kind === 'id') aliases.set(tokens[k - 2]!.value, []);
    if (t.kind === 'id' && t.value === 'default' && tokens[k - 1]?.value === 'export') {
      // export default [async] function [name](param  |  export default [async] (param) =>  |  export default [async] param =>
      let j = k + 1;
      if (tokens[j]?.value === 'async') j++;
      if (tokens[j]?.value === 'function') { j++; if (tokens[j]?.kind === 'id' && tokens[j + 1]?.value === '(') j++; }
      if (tokens[j]?.value === '(' && tokens[j + 1]?.kind === 'id') aliases.set(tokens[j + 1]!.value, []);
      else if (tokens[j]?.kind === 'id' && tokens[j + 1]?.value === '=>') aliases.set(tokens[j]!.value, []);
    }
    if (t.kind === 'id' && t.value === 'function' && tokens[k + 1]?.value === 'activate' && tokens[k + 2]?.value === '(' && tokens[k + 3]?.kind === 'id') aliases.set(tokens[k + 3]!.value, []);
  }

  /** Follows a member chain from token `k` (an identifier that is a root or alias). */
  const chain = (k: number, base: Path, report: boolean): { path: Path | null; end: number } => {
    let path: Path | null = [...base];
    let j = k + 1;
    for (;;) {
      const t = tokens[j];
      if (!t) break;
      if ((t.value === '.' || t.value === '?.') && tokens[j + 1]?.kind === 'id') {
        const name = tokens[j + 1]!;
        if (path) {
          path = [...path, name.value];
          const member = BY_PATH.get(path.join('.'));
          if (member && report) warn(member, name, base.length ? 'alias' : 'static');
          if (!relevant(path)) path = null;
        }
        j += 2;
        continue;
      }
      // `x?.[...]` and `x[...]`
      const open = t.value === '[' ? j : t.value === '?.' && tokens[j + 1]?.value === '[' ? j + 1 : -1;
      if (open >= 0) {
        const close = matching(tokens, open, '[', ']');
        const key = tokens[open + 1];
        const literal = close === open + 2 && (key?.kind === 'str' || (key?.kind === 'tmpl' && key.value !== '')) ? key.value : null;
        if (path) {
          if (literal !== null) {
            path = [...path, literal];
            const member = BY_PATH.get(path.join('.'));
            if (member && report) warn(member, key!, 'computed');
            if (!relevant(path)) path = null;
          } else {
            for (const member of DYNAMIC_PARENTS.get(path.join('.')) ?? []) if (report) warn(member, tokens[open]!, 'dynamic');
            path = null;
          }
        }
        j = close + 1;
        continue;
      }
      if (t.value === '(' || (t.value === '?.' && tokens[j + 1]?.value === '(')) {
        const open = t.value === '(' ? j : j + 1;
        const close = matching(tokens, open, '(', ')');
        if (path && path[path.length - 1] && (path[path.length - 1] === 'get' || path[path.length - 1] === 'register') && path[path.length - 2] === 'services' && path.length === 2) {
          const arg = tokens[open + 1];
          const name = arg?.kind === 'str' || arg?.kind === 'tmpl' ? arg.value : arg?.kind === 'id' ? strings.get(arg.value) : undefined;
          for (const member of SERVICE) if (member.service === name && report) warn(member, arg!, arg?.kind === 'id' ? 'alias' : 'static');
        }
        if (path) {
          path = [...path, '()'];
          if (!relevant(path)) path = null;
        }
        j = close + 1;
        continue;
      }
      if (t.value === '!') { j++; continue; } // a non-null assertion (`!=` is its own token)
      // Type arguments of a call: `get<Raster>('raster')`.
      if (t.value === '<') {
        const end = typeArguments(tokens, j);
        if (end > 0 && tokens[end + 1]?.value === '(') { j = end + 1; continue; }
      }
      break;
    }
    return { path, end: j };
  };

  /** The value an expression starting at `k` resolves to, if it is an API path. */
  const valueAt = (k: number): Path | null => {
    let j = k;
    while (tokens[j]?.value === 'await' || tokens[j]?.value === '(') j++;
    const t = tokens[j];
    if (t?.kind !== 'id') return null;
    const base = aliases.get(t.value);
    if (!base || tokens[j - 1]?.value === '.') return null;
    return chain(j, base, false).path;
  };

  // Two passes collect aliases (a later alias may be declared from an earlier one).
  for (let pass = 0; pass < 2; pass++) {
    for (let k = 0; k < tokens.length; k++) {
      const t = tokens[k]!;
      const declared = t.value === 'const' || t.value === 'let' || t.value === 'var';
      if (declared && tokens[k + 1]?.kind === 'id' && tokens[k + 2]?.value === '=' ) {
        const value = tokens[k + 3];
        if (value?.kind === 'str' && (tokens[k + 4]?.value === ';' || tokens[k + 4]?.value === undefined || tokens[k + 4]?.line !== value.line)) strings.set(tokens[k + 1]!.value, value.value);
        const path = valueAt(k + 3);
        if (path) aliases.set(tokens[k + 1]!.value, path);
      }
      if (declared && tokens[k + 1]?.value === ':' ) continue;
      // `const { a, b: c, d: { e } } = <api path>` (also `let`, `var`, and Svelte `$props()` is ignored)
      if (declared && tokens[k + 1]?.value === '{') {
        const close = matching(tokens, k + 1, '{', '}');
        if (tokens[close + 1]?.value !== '=') continue;
        const source = valueAt(close + 2);
        if (!source) continue;
        destructure(tokens, k + 1, close, source, aliases, pass === 1 ? warn : null);
      }
    }
  }

  // Uses.
  for (let k = 0; k < tokens.length; k++) {
    const t = tokens[k]!;
    if (t.kind !== 'id') continue;
    const base = aliases.get(t.value);
    if (!base) continue;
    const prev = tokens[k - 1];
    if (prev && (prev.value === '.' || prev.value === '?.')) continue; // a property named like a root
    if (prev?.value === ':' && tokens[k - 2]?.value === '{') continue;
    // A bare alias of a deprecated member: `const s = api.render.snapshot; s()` was reported at its declaration.
    chain(k, base, true);
  }
  return warnings;
}

function destructure(tokens: Token[], open: number, close: number, source: Path, aliases: Map<string, Path>,
  warn: ((member: DeprecatedMember, token: Token, access: DeprecationAccess) => void) | null): void {
  let j = open + 1;
  while (j < close) {
    const t = tokens[j]!;
    if (t.value === '...' ) { j += 2; continue; }
    if (t.kind !== 'id' && t.kind !== 'str') { j++; continue; }
    const key = t.value;
    const path = [...source, key];
    const member = BY_PATH.get(path.join('.'));
    if (member) warn?.(member, t, 'destructured');
    if (tokens[j + 1]?.value === ':' && tokens[j + 2]?.value === '{') {
      const inner = matching(tokens, j + 2, '{', '}');
      destructure(tokens, j + 2, inner, path, aliases, warn);
      j = inner + 1;
      continue;
    }
    if (tokens[j + 1]?.value === ':' && tokens[j + 2]?.kind === 'id') {
      if (relevant(path)) aliases.set(tokens[j + 2]!.value, path);
      j += 3;
      continue;
    }
    if (t.kind === 'id' && relevant(path)) aliases.set(key, path);
    j++;
  }
}

/** The `>` closing type arguments opened at `open`, or -1 when it is not a type argument list. */
function typeArguments(tokens: Token[], open: number): number {
  let depth = 0;
  for (let j = open; j < tokens.length && j < open + 64; j++) {
    const v = tokens[j]!.value;
    if (v === '<') depth++;
    else if (v === '>') { if (--depth === 0) return j; }
    else if (tokens[j]!.kind === 'punct' && !['.', ',', '[', ']', '|', '&', '{', '}', ':', ';', '?', '(', ')', '=>'].includes(v)) return -1;
  }
  return -1;
}

function matching(tokens: Token[], open: number, left: string, right: string): number {
  let depth = 0;
  for (let j = open; j < tokens.length; j++) {
    if (tokens[j]!.value === left) depth++;
    else if (tokens[j]!.value === right && --depth === 0) return j;
  }
  return tokens.length - 1;
}
