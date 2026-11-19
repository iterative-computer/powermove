/* Reads token usage out of OpenAI-compatible chat-completion responses, both
   plain JSON and SSE streams. Anthropic's and OpenAI's chat-completions
   endpoints report usage the same way when asked with stream_options.include_usage. */
import type { TokenUsage } from './plans';

interface UsageJson { prompt_tokens?: number; completion_tokens?: number; prompt_tokens_details?: { cached_tokens?: number; cache_creation_tokens?: number }; cache_creation_input_tokens?: number; cache_read_input_tokens?: number }

export function usageFromJson(usage: UsageJson | null | undefined): TokenUsage | null {
  if (!usage || typeof usage.prompt_tokens !== 'number') return null;
  const cacheRead = usage.prompt_tokens_details?.cached_tokens ?? usage.cache_read_input_tokens ?? 0;
  const cacheWrite = usage.prompt_tokens_details?.cache_creation_tokens ?? usage.cache_creation_input_tokens ?? 0;
  // prompt_tokens includes cached tokens; bill them at their own rate, not twice.
  return { input: Math.max(0, usage.prompt_tokens - cacheRead - cacheWrite), output: usage.completion_tokens ?? 0, cacheRead, cacheWrite };
}

/** When an upstream response carries no usage, charge a conservative estimate (4 chars ≈ 1 token, output priced on every byte seen). */
export function estimateUsage(requestBytes: number, responseBytes: number): TokenUsage {
  return { input: Math.ceil(requestBytes / 3), output: Math.ceil(responseBytes / 3) };
}

/** A pass-through transform that watches SSE `data:` lines for a usage object. */
export function meteredStream(onDone: (usage: TokenUsage | null, bytes: number) => Promise<void>): TransformStream<Uint8Array, Uint8Array> {
  const decoder = new TextDecoder();
  let buffer = '', bytes = 0, usage: TokenUsage | null = null;
  const scan = (line: string) => {
    if (!line.startsWith('data:')) return;
    const payload = line.slice(5).trim();
    if (!payload || payload === '[DONE]' || !payload.includes('"usage"')) return;
    try { usage = usageFromJson(JSON.parse(payload).usage) ?? usage; } catch { /* partial or foreign line */ }
  };
  return new TransformStream({
    transform(chunk, controller) {
      bytes += chunk.byteLength;
      controller.enqueue(chunk);
      buffer += decoder.decode(chunk, { stream: true });
      let nl: number;
      while ((nl = buffer.indexOf('\n')) >= 0) { scan(buffer.slice(0, nl).trimEnd()); buffer = buffer.slice(nl + 1); }
    },
    async flush() { if (buffer) scan(buffer.trim()); await onDone(usage, bytes); },
  });
}
