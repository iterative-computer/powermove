import { LIMITS, type CodexTraceEvent } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';

const MAX_LINE_BYTES = 1024 * 1024;

export interface ClaudeEventCallbacks {
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
  onSessionId?: (sessionId: string) => void;
  onWarning?: (message: string) => void;
}

function normalizedText(value: unknown, limit: number): string {
  return typeof value === 'string'
    ? value.replace(/[\u0000-\u001f\u007f-\u009f]/gu, ' ').replace(/\s+/gu, ' ').trim().slice(0, limit)
    : '';
}

export class ClaudeEventParser {
  private readonly decoder = new TextDecoder('utf-8');
  private pending = '';
  private finished = false;
  private sessionId: string | null = null;
  private structuredOutput: unknown = undefined;
  private resultText = '';
  private resultError: string | null = null;

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

    if (event.type === 'assistant' && isRecord(event.message) && Array.isArray(event.message.content)) {
      for (const block of event.message.content) this.assistantBlock(block);
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

  private assistantBlock(value: unknown): void {
    if (!isRecord(value)) return;
    if (value.type === 'text') {
      const text = normalizedText(value.text, LIMITS.codexTraceChars);
      if (!text) return;
      this.callbacks.onTrace?.({ kind: 'answer', text });
      this.callbacks.onProgress?.(text.slice(0, LIMITS.codexProgressChars));
      return;
    }
    if (value.type === 'thinking') {
      const text = normalizedText(value.thinking, LIMITS.codexTraceChars);
      if (text) this.callbacks.onTrace?.({ kind: 'thought', text });
      return;
    }
    if (value.type !== 'tool_use' || !isString(value.id, 120) || !isString(value.name, 80)) return;
    const name = normalizedText(value.name, 80) || 'tool';
    this.callbacks.onTrace?.({
      kind: 'tool-start', itemId: value.id, toolName: name.toLowerCase(), label: name
    });
    this.callbacks.onProgress?.(`Using ${name}…`);
  }

  private toolResult(value: unknown): void {
    if (!isRecord(value) || value.type !== 'tool_result' || !isString(value.tool_use_id, 120)) return;
    this.callbacks.onTrace?.({
      kind: 'tool-end', itemId: value.tool_use_id, isError: value.is_error === true
    });
  }
}
