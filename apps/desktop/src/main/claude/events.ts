import { LIMITS, type CodexTraceEvent } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { fragmentText, humanLabel, outputExcerpt, toolDetail } from '../agent-tools/trace-format';

const MAX_LINE_BYTES = 1024 * 1024;
const MAX_TOOL_BLOCKS = 64;
const MAX_TOOL_INPUT_BYTES = 64 * 1024;
const MAX_PROSE_CHARS = 30_000;
const MAX_RECENT_MESSAGES = 8;

export interface ClaudeEventCallbacks {
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
  onSessionId?: (sessionId: string) => void;
  onWarning?: (message: string) => void;
  projectCwd?: string;
}

interface StreamedToolBlock {
  id: string;
  name: string;
  partialJson: string;
  detailUnavailable: boolean;
}

interface StreamedAssistantMessage {
  tools: Map<number, StreamedToolBlock>;
  prose: Map<number, { kind: 'answer' | 'thought'; text: string }>;
  completed: Set<string>;
}

function normalizedText(value: unknown, limit: number): string {
  return typeof value === 'string'
    ? value.replace(/[\u0000-\u001f\u007f-\u009f]/gu, ' ').replace(/\s+/gu, ' ').trim().slice(0, limit)
    : '';
}

function blockIndex(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/* Prose keeps its shape. Answers and reasoning are rendered as paragraphs and
   lists, so line structure is content — only control characters and trailing
   padding are stripped. Labels and progress lines still use `normalizedText`,
   which flattens everything to a single line. */
function normalizedProse(value: unknown, limit: number): string {
  if (typeof value !== 'string') return '';
  return value
    .replace(/\r\n?/gu, '\n')
    .replace(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/gu, ' ')
    .replace(/ *\n/gu, '\n')
    .replace(/\n{3,}/gu, '\n\n')
    .trim()
    .slice(0, limit);
}

export class ClaudeEventParser {
  private readonly decoder = new TextDecoder('utf-8');
  private pending = '';
  private finished = false;
  private sessionId: string | null = null;
  private structuredOutput: unknown = undefined;
  private resultText = '';
  private resultError: string | null = null;
  private streamedMessage: StreamedAssistantMessage | null = null;
  private lastStreamedMessage: StreamedAssistantMessage | null = null;
  private readonly recentMessages = new Map<string, StreamedAssistantMessage>();

  constructor(private readonly callbacks: ClaudeEventCallbacks = {}) {}

  push(chunk: Uint8Array): void {
    if (this.finished || chunk.byteLength === 0) return;
    this.consume(this.decoder.decode(chunk, { stream: true }));
  }

  finish(): void {
    if (this.finished) return;
    this.finished = true;
    this.consume(this.decoder.decode());
    if (this.pending.trim()) this.line(this.pending);
    this.pending = '';
  }

  get output(): unknown { return this.structuredOutput; }
  get text(): string { return this.resultText; }
  get error(): string | null { return this.resultError; }
  get capturedSessionId(): string | null { return this.sessionId; }

  private consume(text: string): void {
    this.pending += text;
    for (;;) {
      const newline = this.pending.indexOf('\n');
      if (newline < 0) break;
      const line = this.pending.slice(0, newline);
      this.pending = this.pending.slice(newline + 1);
      if (Buffer.byteLength(line, 'utf8') > MAX_LINE_BYTES) {
        this.callbacks.onWarning?.('Claude emitted an oversized event that Powermove ignored.');
      } else {
        this.line(line);
      }
    }
    if (Buffer.byteLength(this.pending, 'utf8') > MAX_LINE_BYTES) {
      this.pending = '';
      this.callbacks.onWarning?.('Claude emitted an oversized event that Powermove ignored.');
    }
  }

  private line(line: string): void {
    let event: unknown;
    try { event = JSON.parse(line); } catch { return; }
    if (!isRecord(event)) return;
    this.captureSession(event.session_id);

    if (event.type === 'stream_event' && isRecord(event.event)) {
      this.streamEvent(event.event);
      return;
    }
    if (event.type === 'assistant' && isRecord(event.message) && Array.isArray(event.message.content)) {
      const streamed = typeof event.message.id === 'string'
        ? this.recentMessages.get(event.message.id) : this.lastStreamedMessage;
      for (const block of event.message.content) {
        if (streamed) this.completeStreamedBlock(streamed, block);
        else this.assistantBlock(block);
      }
      return;
    }
    if (event.type === 'user' && isRecord(event.message) && Array.isArray(event.message.content)) {
      for (const block of event.message.content) this.toolResult(block);
      return;
    }
    if (event.type === 'result') {
      this.structuredOutput = event.structured_output;
      this.resultText = isString(event.result) ? event.result : '';
      if (event.is_error === true || event.subtype === 'error') {
        this.resultError = normalizedText(event.result, 4_000) || 'Claude generation failed.';
      }
    }
  }

  private captureSession(value: unknown): void {
    if (this.sessionId !== null || !isString(value, 200) || !value.trim()) return;
    this.sessionId = value.trim();
    this.callbacks.onSessionId?.(this.sessionId);
  }

  private streamEvent(event: Record<string, unknown>): void {
    if (event.type === 'message_start') {
      this.streamedMessage = { tools: new Map(), prose: new Map(), completed: new Set() };
      this.lastStreamedMessage = this.streamedMessage;
      if (isRecord(event.message) && isString(event.message.id, 200)) {
        this.recentMessages.set(event.message.id, this.streamedMessage);
        while (this.recentMessages.size > MAX_RECENT_MESSAGES) {
          this.recentMessages.delete(this.recentMessages.keys().next().value!);
        }
      }
      return;
    }

    const current = this.streamedMessage;
    if (!current) return;

    if (event.type === 'content_block_start') {
      const index = blockIndex(event.index);
      const block = event.content_block;
      if (index === null || !isRecord(block)) return;
      if (block.type === 'text' || block.type === 'thinking') {
        this.streamProse(current, index, block.type === 'text' ? 'answer' : 'thought',
          block.type === 'text' ? block.text : block.thinking);
        return;
      }
      if (block.type !== 'tool_use') return;
      if (!isString(block.id, 120) || !isString(block.name, 80)) return;
      if (!current.tools.has(index) && current.tools.size >= MAX_TOOL_BLOCKS) return;
      const name = normalizedText(block.name, 80) || 'tool';
      current.tools.set(index, { id: block.id, name, partialJson: '', detailUnavailable: false });
      this.callbacks.onTrace?.({
        kind: 'tool-start',
        itemId: block.id,
        toolName: name.toLowerCase(),
        label: humanLabel(name)
      });
      return;
    }

    if (event.type === 'content_block_delta' && isRecord(event.delta)) {
      if (event.delta.type === 'text_delta') {
        this.streamProse(current, blockIndex(event.index), 'answer', event.delta.text);
        return;
      }
      if (event.delta.type === 'thinking_delta') {
        this.streamProse(current, blockIndex(event.index), 'thought', event.delta.thinking);
        return;
      }
      if (event.delta.type !== 'input_json_delta') return;
      const index = blockIndex(event.index);
      const tool = index === null ? undefined : current.tools.get(index);
      if (!tool || tool.detailUnavailable || !isString(event.delta.partial_json)) return;
      if (Buffer.byteLength(tool.partialJson, 'utf8') + Buffer.byteLength(event.delta.partial_json, 'utf8') > MAX_TOOL_INPUT_BYTES) {
        tool.detailUnavailable = true;
        tool.partialJson = '';
        return;
      }
      tool.partialJson += event.delta.partial_json;
      return;
    }

    if (event.type === 'content_block_stop') {
      const index = blockIndex(event.index);
      const tool = index === null ? undefined : current.tools.get(index);
      if (!tool) return;
      current.tools.delete(index!);
      let input: unknown;
      if (!tool.detailUnavailable) {
        try { input = JSON.parse(tool.partialJson); } catch { input = undefined; }
      }
      const detail = input === undefined ? '' : toolDetail(tool.name, input, this.callbacks.projectCwd);
      this.callbacks.onTrace?.({
        kind: 'tool-start',
        itemId: tool.id,
        toolName: tool.name.toLowerCase(),
        label: humanLabel(tool.name),
        ...(detail ? { detail } : {})
      });
      this.callbacks.onProgress?.(`Using ${humanLabel(tool.name)}…`);
      return;
    }

    if (event.type === 'message_stop') this.finishStreamedMessage();
  }

  private emitProse(kind: 'answer' | 'thought', value: string): void {
    const bounded = value.slice(0, MAX_PROSE_CHARS);
    // The IPC limit is per event, not per reply. Cutting off a complete block
    // can discard its closing Markdown markers and leave raw syntax visible.
    for (let offset = 0; offset < bounded.length;) {
      let end = Math.min(offset + LIMITS.codexTraceChars, bounded.length);
      if (end < bounded.length && /[\uD800-\uDBFF]/u.test(bounded[end - 1]!)) end -= 1;
      const text = fragmentText(bounded.slice(offset, end));
      if (text) this.callbacks.onTrace?.({ kind, text });
      offset = end;
    }
  }

  private streamProse(message: StreamedAssistantMessage, index: number | null,
    kind: 'answer' | 'thought', value: unknown): void {
    if (typeof value !== 'string') return;
    if (index !== null && (message.prose.has(index) || message.prose.size < MAX_TOOL_BLOCKS)) {
      const previous = message.prose.get(index);
      message.prose.set(index, { kind, text: `${previous?.text ?? ''}${value}`.slice(0, MAX_PROSE_CHARS) });
    }
    this.emitProse(kind, value);
  }

  private completeStreamedBlock(message: StreamedAssistantMessage, value: unknown): void {
    if (!isRecord(value) || (value.type !== 'text' && value.type !== 'thinking')) return;
    const kind = value.type === 'text' ? 'answer' : 'thought';
    const raw = kind === 'answer' ? value.text : value.thinking;
    if (typeof raw !== 'string') return;
    const text = raw.slice(0, MAX_PROSE_CHARS);
    const key = `${kind}:${text}`;
    if (message.completed.has(key)) return;
    if (message.completed.size >= MAX_TOOL_BLOCKS) return;
    const candidates = [...message.prose.values()].filter(block => block.kind === kind
      && !message.completed.has(`${kind}:${block.text}`));
    message.completed.add(key);
    // Claude may send one assistant event per block. Match the actual content,
    // not the position in that event or whether some other block had deltas.
    const seen = candidates.find(block => block.text === text)
      ?? candidates.find(block => block.text && text.startsWith(block.text));
    const missing = seen ? text.slice(seen.text.length) : text;
    this.emitProse(kind, missing);
    if (seen) seen.text = text;
    if (kind === 'answer') this.streamedAssistantProgress(value);
  }

  private finishStreamedMessage(): void {
    this.streamedMessage?.tools.clear();
    this.streamedMessage = null;
  }

  private streamedAssistantProgress(value: unknown): void {
    if (!isRecord(value) || value.type !== 'text') return;
    const text = normalizedText(value.text, LIMITS.codexTraceChars);
    if (text) this.callbacks.onProgress?.(text.slice(0, LIMITS.codexProgressChars));
  }

  private assistantBlock(value: unknown): void {
    if (!isRecord(value)) return;
    if (value.type === 'text') {
      const text = normalizedProse(value.text, MAX_PROSE_CHARS);
      if (!text) return;
      this.emitProse('answer', text);
      const progress = normalizedText(value.text, LIMITS.codexProgressChars);
      if (progress) this.callbacks.onProgress?.(progress);
      return;
    }
    if (value.type === 'thinking') {
      const text = normalizedProse(value.thinking, MAX_PROSE_CHARS);
      this.emitProse('thought', text);
      return;
    }
    if (value.type !== 'tool_use' || !isString(value.id, 120) || !isString(value.name, 80)) return;
    const name = normalizedText(value.name, 80) || 'tool';
    const detail = toolDetail(name, value.input, this.callbacks.projectCwd);
    this.callbacks.onTrace?.({
      kind: 'tool-start',
      itemId: value.id,
      toolName: name.toLowerCase(),
      label: humanLabel(name),
      ...(detail ? { detail } : {})
    });
    this.callbacks.onProgress?.(`Using ${humanLabel(name)}…`);
  }

  private toolResult(value: unknown): void {
    if (!isRecord(value) || value.type !== 'tool_result' || !isString(value.tool_use_id, 120)) return;
    let content = '';
    if (isString(value.content)) content = value.content;
    else if (Array.isArray(value.content)) {
      content = value.content
        .filter((block): block is Record<string, unknown> => isRecord(block) && block.type === 'text' && isString(block.text))
        .map((block) => block.text as string)
        .join('\n');
    }
    const output = outputExcerpt(content || value);
    this.callbacks.onTrace?.({
      kind: 'tool-end',
      itemId: value.tool_use_id,
      isError: value.is_error === true,
      ...(output ? { output } : {})
    });
  }
}
