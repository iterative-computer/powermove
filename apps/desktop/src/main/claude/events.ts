import { LIMITS, type CodexTraceEvent } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { fragmentText, humanLabel, outputExcerpt, toolDetail } from '../agent-tools/trace-format';

const MAX_LINE_BYTES = 1024 * 1024;
const MAX_TOOL_BLOCKS = 64;
const MAX_TOOL_INPUT_BYTES = 64 * 1024;

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
  /* A streamed message can finish without any text deltas. In that case its
     complete assistant block is the only copy of the answer. */
  private streamingMode = false;
  private streamedAnswer = false;
  private streamedThought = false;
  private readonly completedBlocks = new Set<string>();

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
      for (const block of event.message.content) {
        if (!this.streamingMode) { this.assistantBlock(block); continue; }
        if (!isRecord(block)) continue;
        if (block.type !== 'text' && block.type !== 'thinking') continue;
        if (block.type === 'text' && this.streamedAnswer) this.streamedAssistantProgress(block);
        if ((block.type === 'text' && this.streamedAnswer)
          || (block.type === 'thinking' && this.streamedThought)) continue;
        const key = `${block.type}:${block.type === 'text' ? block.text : block.thinking}`;
        if (this.completedBlocks.has(key)) continue;
        this.completedBlocks.add(key);
        this.assistantBlock(block);
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
      this.streamingMode = true;
      this.streamedAnswer = false;
      this.streamedThought = false;
      this.completedBlocks.clear();
      this.streamedMessage = { tools: new Map() };
      return;
    }

    const current = this.streamedMessage;
    if (!current) return;

    if (event.type === 'content_block_start') {
      const index = blockIndex(event.index);
      const block = event.content_block;
      if (index === null || !isRecord(block) || block.type !== 'tool_use') return;
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
        if (typeof event.delta.text === 'string' && event.delta.text) this.streamedAnswer = true;
        this.emitFragment('answer', event.delta.text);
        return;
      }
      if (event.delta.type === 'thinking_delta') {
        if (typeof event.delta.thinking === 'string' && event.delta.thinking) this.streamedThought = true;
        this.emitFragment('thought', event.delta.thinking);
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

  private emitFragment(kind: 'answer' | 'thought', value: unknown): void {
    const text = fragmentText(value).slice(0, LIMITS.codexTraceChars);
    if (text) this.callbacks.onTrace?.({ kind, text });
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
      const text = normalizedProse(value.text, LIMITS.codexTraceChars);
      if (!text) return;
      this.callbacks.onTrace?.({ kind: 'answer', text });
      const progress = normalizedText(value.text, LIMITS.codexProgressChars);
      if (progress) this.callbacks.onProgress?.(progress);
      return;
    }
    if (value.type === 'thinking') {
      const text = normalizedProse(value.thinking, LIMITS.codexTraceChars);
      if (text) this.callbacks.onTrace?.({ kind: 'thought', text });
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
