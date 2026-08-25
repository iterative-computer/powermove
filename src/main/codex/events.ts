import { LIMITS } from '../../shared/ipc';

export const MAX_CODEX_EVENT_LINE_BYTES = 1024 * 1024;

export interface CodexEventParserCallbacks {
  onProgress?: (text: string) => void;
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

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

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

  if (typeof item.text !== 'string') return null;
  let displayText = item.text;

  // Codex sometimes places a small user-facing status object in `text`.
  // Read only the explicitly allowed presentation fields from such objects.
  try {
    const structured: unknown = JSON.parse(displayText);
    if (isRecord(structured)) {
      displayText =
        DISPLAY_FIELDS.map((field) => structured[field]).find(
          (value): value is string => typeof value === 'string' && value.length > 8
        ) ?? '';
    } else {
      displayText = '';
    }
  } catch {
    // Plain text reasoning and agent messages remain valid progress updates.
  }

  const summary = displayText.replace(/\s+/g, ' ').trim();
  return summary.length > 0 ? summary.slice(0, LIMITS.codexProgressChars) : null;
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
  }

  private warnLongLine(): void {
    this.callbacks.onWarning?.(
      `Dropped Codex NDJSON line larger than ${MAX_CODEX_EVENT_LINE_BYTES} bytes.`
    );
  }
}

// The descriptive alias makes the class convenient at stdout plumbing sites.
export { CodexEventParser as CodexNdjsonParser };
