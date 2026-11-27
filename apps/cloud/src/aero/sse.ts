/* AI SDK stream parts out as OpenAI Chat Completions SSE chunks, in the subset fx's reducer accepts
   (chat_completions_protocol.zig): one choice at index 0; role, content, reasoning_content, reasoning_details
   and tool_calls deltas; a finish_reason chunk that agrees with the tool calls; then a chunk with no choices
   and the usage; then `data: [DONE]`. */
import type { LanguageModelUsage, TextStreamPart, ToolSet } from 'ai';
import type { Provider } from './catalogue';
import { type ReasoningDetailItem, replayMetadata } from './translate';

export interface Usage { input: number; cacheRead: number; cacheWrite: number; output: number; reasoning: number }
export const usageOf = (usage: LanguageModelUsage): Usage => ({
  input: usage.inputTokens ?? 0,
  cacheRead: usage.inputTokenDetails?.cacheReadTokens ?? 0,
  cacheWrite: usage.inputTokenDetails?.cacheWriteTokens ?? 0,
  output: usage.outputTokens ?? 0,
  reasoning: usage.outputTokenDetails?.reasoningTokens ?? 0,
});

type Delta = Record<string, unknown>;

export class ChunkEncoder {
  private role = false;
  private readonly tools = new Map<string, { index: number; arguments: string }>();
  private readonly reasoning = new Map<string, { text: string; metadata: Record<string, unknown> }>();
  finishReason: 'stop' | 'tool_calls' | 'length' | 'content_filter' | null = null;
  usage: Usage | null = null;
  failed = false;

  constructor(private readonly id: string, private readonly model: string, private readonly provider: Provider, private readonly created = Math.floor(Date.now() / 1000)) {}

  private chunk(delta: Delta, extra: Record<string, unknown> = {}): string {
    if (!this.role) { delta = { role: 'assistant', ...delta }; this.role = true; }
    return frame({ id: this.id, object: 'chat.completion.chunk', created: this.created, model: this.model, choices: [{ index: 0, delta, finish_reason: null, ...extra }] });
  }

  /** The SSE frames one stream part becomes (often none). */
  accept(part: TextStreamPart<ToolSet>): string[] {
    switch (part.type) {
      case 'text-delta':
        if (!part.text) return [];
        return [this.chunk({ content: part.text })];
      case 'reasoning-start':
        this.reasoning.set(part.id, { text: '', metadata: { ...part.providerMetadata?.[this.provider] } });
        return [];
      case 'reasoning-delta': {
        const block = this.reasoning.get(part.id) ?? { text: '', metadata: {} };
        block.text += part.text;
        Object.assign(block.metadata, part.providerMetadata?.[this.provider]);
        this.reasoning.set(part.id, block);
        if (!part.text) return [];
        return [this.chunk({ reasoning_content: part.text })];
      }
      case 'reasoning-end': {
        const block = this.reasoning.get(part.id) ?? { text: '', metadata: {} };
        this.reasoning.delete(part.id);
        Object.assign(block.metadata, part.providerMetadata?.[this.provider]);
        const metadata = replayMetadata(this.provider, block.metadata);
        if (Object.keys(metadata).length === 0) return [];
        const item: ReasoningDetailItem = { type: 'reasoning.aero', provider: this.provider, text: block.text, metadata };
        return [this.chunk({ reasoning_details: [item] })];
      }
      case 'tool-input-start': {
        const index = this.tools.size;
        this.tools.set(part.id, { index, arguments: '' });
        return [this.chunk({ tool_calls: [{ index, id: part.id, type: 'function', function: { name: part.toolName, arguments: '' } }] })];
      }
      case 'tool-input-delta': {
        const call = this.tools.get(part.id);
        if (!call || !part.delta) return [];
        call.arguments += part.delta;
        return [this.chunk({ tool_calls: [{ index: call.index, function: { arguments: part.delta } }] })];
      }
      case 'tool-call': {
        const call = this.tools.get(part.toolCallId);
        const input = JSON.stringify(part.input ?? {});
        if (!call) {
          // Not streamed: the whole call in one delta.
          const index = this.tools.size;
          this.tools.set(part.toolCallId, { index, arguments: input });
          return [this.chunk({ tool_calls: [{ index, id: part.toolCallId, type: 'function', function: { name: part.toolName, arguments: input } }] })];
        }
        if (call.arguments.trim() !== '') return [];
        call.arguments = input;
        return [this.chunk({ tool_calls: [{ index: call.index, function: { arguments: input } }] })];
      }
      case 'finish': {
        this.usage = usageOf(part.totalUsage);
        const reason = part.finishReason;
        this.finishReason = this.tools.size > 0 ? 'tool_calls' : reason === 'length' ? 'length' : reason === 'content-filter' ? 'content_filter' : 'stop';
        return [];
      }
      case 'error':
        this.failed = true;
        return [];
      default:
        return [];
    }
  }

  /** The closing frames: finish_reason, the usage, [DONE]. */
  close(usage: Usage): string[] {
    const reason = this.finishReason ?? (this.tools.size > 0 ? 'tool_calls' : 'stop');
    const input = Math.max(usage.input, usage.cacheRead + usage.cacheWrite);
    const output = Math.max(usage.output, usage.reasoning);
    return [
      this.chunk({}, { finish_reason: reason }),
      frame({
        id: this.id, object: 'chat.completion.chunk', created: this.created, model: this.model, choices: [],
        usage: {
          prompt_tokens: input, completion_tokens: output, total_tokens: input + output,
          prompt_tokens_details: { cached_tokens: usage.cacheRead, cache_write_tokens: usage.cacheWrite },
          completion_tokens_details: { reasoning_tokens: usage.reasoning },
        },
      }),
      'data: [DONE]\n\n',
    ];
  }

  /** A failure in the stream: a code only, never the provider's text. */
  static error(code: string): string { return frame({ error: { code } }); }
}

const frame = (value: unknown) => `data: ${JSON.stringify(value)}\n\n`;
