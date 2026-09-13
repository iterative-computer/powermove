import path from 'node:path';

import { LIMITS } from '../../shared/ipc';

type UnknownRecord = Record<string, unknown>;

const TOOL_LABELS: Readonly<Record<string, string>> = {
  read: 'Read',
  edit: 'Edit',
  multiedit: 'Edit',
  write: 'Write',
  bash: 'Run',
  grep: 'Search',
  glob: 'Search',
  ls: 'List',
  websearch: 'Search web',
  webfetch: 'Fetch',
  agent: 'Agent',
  task: 'Agent',
  todowrite: 'Plan',
  notebookedit: 'Edit notebook'
};

const FILE_TOOLS = new Set(['read', 'edit', 'write', 'multiedit', 'notebookedit']);
const DETAIL_KEYS = ['command', 'file_path', 'path', 'query', 'url', 'description'] as const;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function truncate(value: string, limit: number): string {
  if (value.length <= limit) return value;
  const sliced = value.slice(0, limit);
  return /[\uD800-\uDBFF]$/u.test(sliced) ? sliced.slice(0, -1) : sliced;
}

function humanise(value: string): string {
  const words = value
    .replace(/([a-z\d])([A-Z])/gu, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/gu, '$1 $2')
    .replace(/[_-]+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim()
    .toLocaleLowerCase();
  return words ? words.charAt(0).toLocaleUpperCase() + words.slice(1) : 'Tool';
}

function mcpToolName(name: string): string | null {
  const match = /^mcp__.+?__(.+)$/iu.exec(name);
  return match?.[1] || null;
}

/** Produce the short action label rendered before a tool's primary detail. */
export function humanLabel(name: string): string {
  const clean = name.replace(/[\u0000-\u001f\u007f-\u009f]/gu, '').trim();
  if (!clean) return 'Tool';
  const mapped = TOOL_LABELS[clean.toLocaleLowerCase()];
  const label = mapped ?? humanise(mcpToolName(clean) ?? clean);
  return truncate(label, 40);
}

function projectPath(value: string, projectCwd?: string): string {
  if (!projectCwd || !path.isAbsolute(value) || !path.isAbsolute(projectCwd)) return value;
  const relative = path.relative(path.resolve(projectCwd), path.resolve(value));
  if (relative === '') return '.';
  if (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)) return relative;
  return value;
}

function stringField(input: UnknownRecord, key: string): string {
  const value = input[key];
  return typeof value === 'string' ? value : '';
}

function oneLine(value: string): string {
  return truncate(
    value
      .replace(/\r\n?/gu, '\n')
      .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/gu, '')
      .replace(/\n/gu, ' ⏎ '),
    LIMITS.codexToolDetailChars
  );
}

/** Extract the most useful bounded argument for a tool activity row. */
export function toolDetail(name: string, input: unknown, projectCwd?: string): string {
  if (!isRecord(input)) return '';
  const key = name.toLocaleLowerCase();
  let detail = '';

  if (key === 'bash') detail = stringField(input, 'command');
  else if (FILE_TOOLS.has(key)) detail = projectPath(stringField(input, 'file_path'), projectCwd);
  else if (key === 'grep') {
    const pattern = stringField(input, 'pattern');
    const searchPath = stringField(input, 'path');
    detail = pattern && searchPath ? `${pattern} in ${projectPath(searchPath, projectCwd)}` : pattern;
  } else if (key === 'glob') detail = stringField(input, 'pattern');
  else if (key === 'websearch') detail = stringField(input, 'query');
  else if (key === 'webfetch') detail = stringField(input, 'url');
  else if (key === 'agent' || key === 'task') detail = stringField(input, 'description');

  if (!detail) {
    for (const candidate of DETAIL_KEYS) {
      detail = stringField(input, candidate);
      if (detail) break;
    }
  }
  if (!detail) {
    for (const value of Object.values(input)) {
      if (typeof value === 'string' && value) { detail = value; break; }
    }
  }
  return oneLine(detail);
}

/** Sanitize one append-only text delta without altering meaningful whitespace. */
export function fragmentText(value: unknown): string {
  if (typeof value !== 'string') return '';
  return truncate(
    value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/gu, ''),
    LIMITS.codexTraceChars
  );
}

function outputText(value: unknown, seen: Set<object>): string {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return value.map((item) => outputText(item, seen)).filter(Boolean).join('\n');
  if (!isRecord(value) || seen.has(value)) return '';
  seen.add(value);

  for (const key of ['error', 'aggregated_output', 'aggregatedOutput', 'output', 'content', 'text', 'message', 'status']) {
    if (!(key in value)) continue;
    const text = outputText(value[key], seen);
    if (text) return text;
  }
  return '';
}

/** Produce a compact tool-result excerpt: at most eight lines and 600 chars. */
export function outputExcerpt(value: unknown): string {
  const text = outputText(value, new Set())
    .replace(/\r\n?/gu, '\n')
    .replace(/\t/gu, ' ')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/gu, '');
  return truncate(text.split('\n').slice(0, 8).join('\n'), LIMITS.codexToolOutputChars);
}
