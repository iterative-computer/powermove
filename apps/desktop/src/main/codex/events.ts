import { LIMITS, type CodexTraceEvent } from '../../shared/ipc';

export const MAX_CODEX_EVENT_LINE_BYTES = 1024 * 1024;

export interface CodexEventParserCallbacks {
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
  onThreadId?: (threadId: string) => void;
  onWarning?: (message: string) => void;
}

const STARTED_ACTIVITIES: Readonly<Record<string, string>> = {
  command_execution: 'Working with project files and shell tools…',
  web_search: 'Researching on the web…',
  computer_use: 'Operating an application on this Mac…',
  image_generation: 'Generating a visual deliverable…',
  file_change: 'Preparing project files…'
};

const DISPLAY_FIELDS = ['message', 'summary', 'critique', 'status'] as const;
const TOOL_ITEM_TYPES = new Set([
  'command_execution',
  'file_change',
  'web_search',
  'image_generation',
  'computer_use',
  'mcp_tool_call'
]);
const TRACE_ITEM_ID_CHARS = 120;
const TRACE_LABEL_CHARS = 120;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

function completedDisplayText(item: Record<string, unknown>, minimumStructuredLength = 0): string | null {
  if (typeof item.text !== 'string') return null;
  let displayText = item.text;

  // Structured output may contain executable or private fields. Only the
  // established presentation whitelist is allowed to cross this boundary.
  try {
    const structured: unknown = JSON.parse(displayText);
    if (!isRecord(structured)) return null;
    displayText =
      DISPLAY_FIELDS.map((field) => structured[field]).find(
        (value): value is string =>
          typeof value === 'string' && value.length > minimumStructuredLength
      ) ?? '';
  } catch {
    // Plain prose is a display payload already.
  }

  return displayText;
}

function labelPart(value: unknown, limit: number): string {
  if (typeof value !== 'string') return '';
  return value
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, limit);
}

function label(value: string): string {
  return labelPart(value, TRACE_LABEL_CHARS);
}

function basename(value: unknown): string {
  const path = labelPart(value, TRACE_LABEL_CHARS);
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? '';
}

function changedFileNames(item: Record<string, unknown>): string[] {
  const values: unknown[] = [];
  if (Array.isArray(item.changes)) {
    for (const change of item.changes) {
      if (typeof change === 'string') values.push(change);
      else if (isRecord(change)) values.push(change.path);
    }
  }
  if (values.length === 0 && Array.isArray(item.files)) values.push(...item.files);
  if (values.length === 0) values.push(item.path);
  return values.map(basename).filter(Boolean).slice(0, 3);
}

function toolStart(item: Record<string, unknown>): CodexTraceEvent | null {
  if (typeof item.id !== 'string' || item.id.length === 0 || typeof item.type !== 'string') return null;
  const itemId = item.id.slice(0, TRACE_ITEM_ID_CHARS);

  switch (item.type) {
    case 'command_execution': {
      const detail = labelPart(item.command, 68);
      return { kind: 'tool-start', itemId, toolName: 'bash', label: label(detail ? `bash · ${detail}` : 'bash') };
    }
    case 'file_change': {
      const detail = changedFileNames(item).join(', ');
      return { kind: 'tool-start', itemId, toolName: 'edit', label: label(detail ? `edit · ${detail}` : 'edit') };
    }
    case 'web_search': {
      const detail = labelPart(item.query, 60);
      return { kind: 'tool-start', itemId, toolName: 'search', label: label(detail ? `search · ${detail}` : 'search') };
    }
    case 'image_generation':
      return { kind: 'tool-start', itemId, toolName: 'image', label: 'image' };
    case 'computer_use':
      return { kind: 'tool-start', itemId, toolName: 'computer', label: 'computer' };
    case 'mcp_tool_call': {
      const tool = labelPart(item.tool, 40) || 'mcp';
      const name = labelPart(item.name, 40) || 'tool';
      return { kind: 'tool-start', itemId, toolName: tool, label: label(`${tool} · ${name}`) };
    }
    default:
      return null;
  }
}

/** Returns the first thread id carried by a thread.started event. */
export function threadIdForCodexEvent(event: unknown): string | null {
  if (!isRecord(event) || event.type !== 'thread.started') return null;
  return typeof event.thread_id === 'string' && event.thread_id.length > 0 ? event.thread_id : null;
}

/**
 * Converts a Codex JSON event into renderer-safe progress copy. Structured
 * agent output is deliberately restricted to known display fields so edit or
 * shell commands can never leak into the progress stream.
 */
export function progressForCodexEvent(event: unknown): string | null {
  if (!isRecord(event) || !isRecord(event.item)) return null;

  const item = event.item;
  const itemType = item.type;
  if (typeof itemType !== 'string') return null;

  if (event.type === 'item.started') {
    if (itemType === 'mcp_tool_call') {
      const rawTool = typeof item.tool === 'string' ? item.tool : typeof item.name === 'string' ? item.name : 'integration';
      return `Using the installed ${rawTool.slice(0, 80)} integration…`;
    }
    return STARTED_ACTIVITIES[itemType] ?? null;
  }

  if (event.type !== 'item.completed' || (itemType !== 'reasoning' && itemType !== 'agent_message')) {
    return null;
  }

  const displayText = completedDisplayText(item, 8);
  if (displayText === null) return null;

  const summary = displayText.replace(/\s+/g, ' ').trim();
  return summary.length > 0 ? summary.slice(0, LIMITS.codexProgressChars) : null;
}

/** Maps a Codex item event to the bounded structured trace contract. */
export function traceForCodexEvent(event: unknown): CodexTraceEvent | null {
  if (!isRecord(event) || !isRecord(event.item)) return null;
  const item = event.item;
  if (typeof item.type !== 'string') return null;

  if (event.type === 'item.started') return toolStart(item);
  if (event.type !== 'item.completed') return null;

  if (item.type === 'reasoning' || item.type === 'agent_message') {
    const rawText = completedDisplayText(item);
    if (rawText === null) return null;
    const text = rawText.trim().slice(0, LIMITS.codexTraceChars);
    if (!text) return null;
    return { kind: item.type === 'reasoning' ? 'thought' : 'answer', text };
  }

  if (!TOOL_ITEM_TYPES.has(item.type) || typeof item.id !== 'string' || item.id.length === 0) return null;
  const status = typeof item.status === 'string' ? item.status.toLowerCase() : '';
  return {
    kind: 'tool-end',
    itemId: item.id.slice(0, TRACE_ITEM_ID_CHARS),
    isError: status === 'failed' || status === 'error'
  };
}

/**
 * Incremental stdout parser for `codex --json`. TextDecoder streaming keeps a
 * split multi-byte UTF-8 sequence intact. The parser can be fed Node Buffers
 * directly because Buffer is a Uint8Array.
 */
export class CodexEventParser {
  private readonly decoder = new TextDecoder('utf-8');
  private pending = '';
  private droppingLongLine = false;
  private finished = false;
  private threadId: string | null = null;

  constructor(private readonly callbacks: CodexEventParserCallbacks = {}) {}

  push(chunk: Uint8Array): void {
    if (this.finished || chunk.byteLength === 0) return;
    this.consumeText(this.decoder.decode(chunk, { stream: true }));
  }

  finish(): void {
    if (this.finished) return;
    this.finished = true;
    this.consumeText(this.decoder.decode());
    if (!this.droppingLongLine && this.pending.length > 0) this.consumeLine(this.pending);
    this.pending = '';
    this.droppingLongLine = false;
  }

  get capturedThreadId(): string | null {
    return this.threadId;
  }

  private consumeText(text: string): void {
    let offset = 0;
    for (;;) {
      const newline = text.indexOf('\n', offset);
      if (newline === -1) break;

      const segment = text.slice(offset, newline);
      if (this.droppingLongLine) {
        this.droppingLongLine = false;
      } else if (Buffer.byteLength(this.pending) + Buffer.byteLength(segment) > MAX_CODEX_EVENT_LINE_BYTES) {
        this.warnLongLine();
      } else {
        this.consumeLine(this.pending + segment);
      }
      this.pending = '';
      offset = newline + 1;
    }

    const tail = text.slice(offset);
    if (tail.length === 0 || this.droppingLongLine) return;
    if (Buffer.byteLength(this.pending) + Buffer.byteLength(tail) > MAX_CODEX_EVENT_LINE_BYTES) {
      this.pending = '';
      this.droppingLongLine = true;
      this.warnLongLine();
      return;
    }
    this.pending += tail;
  }

  private consumeLine(line: string): void {
    if (line.trim().length === 0) return;

    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      return;
    }

    if (this.threadId === null) {
      const threadId = threadIdForCodexEvent(event);
      if (threadId !== null) {
        this.threadId = threadId;
        this.callbacks.onThreadId?.(threadId);
      }
    }

    const progress = progressForCodexEvent(event);
    if (progress !== null) this.callbacks.onProgress?.(progress);
    const trace = traceForCodexEvent(event);
    if (trace !== null) this.callbacks.onTrace?.(trace);
  }

  private warnLongLine(): void {
    this.callbacks.onWarning?.(
      `Dropped Codex NDJSON line larger than ${MAX_CODEX_EVENT_LINE_BYTES} bytes.`
    );
  }
}

// The descriptive alias makes the class convenient at stdout plumbing sites.
export { CodexEventParser as CodexNdjsonParser };
