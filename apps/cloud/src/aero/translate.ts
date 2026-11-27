/* OpenAI Chat Completions in, AI SDK call out. The request shape is what fx's `openai-chat-completions`
   connection writes (apps/native/vendor/fx/src/gateway/chat_completions_protocol.zig): system lane first,
   assistant tool calls, tool results by id, data-URL images, reasoning replay fields, function tools. */
import { z } from 'zod';
import { jsonSchema, tool, type AssistantModelMessage, type JSONValue, type ModelMessage, type SystemModelMessage, type ToolSet, type UserModelMessage } from 'ai';
import type { AeroModel, Provider } from './catalogue';

const Text = z.object({ type: z.literal('text'), text: z.string() });
const Image = z.object({ type: z.literal('image_url'), image_url: z.object({ url: z.string(), detail: z.string().optional() }) });
const Content = z.union([z.string(), z.array(z.union([Text, Image]))]);
const ToolCall = z.object({ id: z.string().min(1).max(256), type: z.literal('function').optional(), function: z.object({ name: z.string().min(1).max(128), arguments: z.string().max(1_048_576) }) });
const ReasoningDetail = z.object({}).passthrough();
const Message = z.discriminatedUnion('role', [
  z.object({ role: z.literal('system'), content: Content }),
  z.object({ role: z.literal('developer'), content: Content }),
  z.object({ role: z.literal('user'), content: Content }),
  z.object({
    role: z.literal('assistant'),
    content: Content.nullable().optional(),
    tool_calls: z.array(ToolCall).max(128).optional(),
    reasoning: z.string().nullable().optional(),
    reasoning_content: z.string().nullable().optional(),
    reasoning_details: z.array(ReasoningDetail).nullable().optional(),
  }),
  z.object({ role: z.literal('tool'), tool_call_id: z.string().min(1).max(256), content: Content }),
]);
const FunctionTool = z.object({
  type: z.literal('function'),
  function: z.object({ name: z.string().regex(/^[A-Za-z0-9_-]{1,128}$/), description: z.string().max(65_536).optional(), parameters: z.record(z.string(), z.unknown()).optional(), strict: z.boolean().nullable().optional() }),
});
export const ChatRequest = z.object({
  model: z.string().min(1).max(128),
  messages: z.array(Message).min(1).max(10_000),
  stream: z.literal(true),
  stream_options: z.object({ include_usage: z.boolean().optional() }).passthrough().optional(),
  tools: z.array(FunctionTool).max(128).optional(),
  tool_choice: z.union([z.enum(['auto', 'none', 'required']), z.object({ type: z.literal('function'), function: z.object({ name: z.string() }) })]).optional(),
  parallel_tool_calls: z.boolean().optional(),
  reasoning_effort: z.string().max(16).nullable().optional(),
  max_completion_tokens: z.number().int().positive().max(1_000_000).optional(),
  max_tokens: z.number().int().positive().max(1_000_000).optional(),
  temperature: z.number().min(0).max(2).optional(),
  top_p: z.number().min(0).max(1).optional(),
  prompt_cache_key: z.string().max(256).optional(),
  user: z.string().max(256).optional(),
}).passthrough();
export type ChatRequest = z.infer<typeof ChatRequest>;

export class UnsupportedRequest extends Error {}

/** One replayable reasoning block, as it travels in `reasoning_details`. Only the provider's own fields that
    carry continuity (a signature, redacted data, an encrypted item) go out, and only those come back. */
export interface ReasoningDetailItem { type: 'reasoning.aero'; provider: Provider; text: string; metadata: Record<string, string> }
const REPLAY_FIELDS: Record<Provider, readonly string[]> = { anthropic: ['signature', 'redactedData'], openai: ['itemId', 'reasoningEncryptedContent'] };
export function replayMetadata(provider: Provider, metadata: Record<string, unknown> | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const field of REPLAY_FIELDS[provider]) {
    const value = metadata?.[field];
    if (typeof value === 'string' && value.length > 0) out[field] = value;
  }
  return out;
}

function text(content: z.infer<typeof Content> | null | undefined): string {
  if (content == null) return '';
  return typeof content === 'string' ? content : content.filter(part => part.type === 'text').map(part => (part as { text: string }).text).join('\n');
}

const IMAGE_TYPES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
const DATA_URL = /^data:([a-z]+\/[a-z0-9.+-]+);base64,([A-Za-z0-9+/=]+)$/i;
function userContent(content: z.infer<typeof Content>): UserModelMessage['content'] {
  if (typeof content === 'string') return content;
  return content.map((part) => {
    if (part.type === 'text') return { type: 'text' as const, text: part.text };
    // Only inline images: the backend never fetches a URL a client names.
    const match = DATA_URL.exec(part.image_url.url);
    if (!match) throw new UnsupportedRequest('image');
    // Images only: a document (a PDF, anything else) is refused, never passed on as a file.
    if (!IMAGE_TYPES.has(match[1]!.toLowerCase())) throw new UnsupportedRequest('file');
    return { type: 'image' as const, image: match[2]!, mediaType: match[1]!.toLowerCase() };
  });
}

function parseArguments(raw: string): Record<string, unknown> {
  if (raw.trim() === '') return {};
  try {
    const value = JSON.parse(raw) as unknown;
    return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
  } catch {
    return {};
  }
}

const EPHEMERAL = { cacheControl: { type: 'ephemeral' as const } };

export interface Prepared {
  instructions: SystemModelMessage[];
  messages: ModelMessage[];
  tools: ToolSet | undefined;
  toolChoice: 'auto' | 'none' | 'required' | { type: 'tool'; toolName: string } | undefined;
  providerOptions: Record<string, Record<string, JSONValue>>;
  maxOutputTokens: number | undefined;
  temperature: number | undefined;
  topP: number | undefined;
}

/** Builds the AI SDK call. `cacheKey` is the thread's stable prompt cache key (OpenAI). */
export function prepare(request: ChatRequest, model: AeroModel, cacheKey: string): Prepared {
  const anthropic = model.provider === 'anthropic';
  const instructions: SystemModelMessage[] = [];
  const messages: ModelMessage[] = [];
  const toolNames = new Map<string, string>();
  for (const message of request.messages) {
    switch (message.role) {
      case 'system':
      case 'developer':
        instructions.push({ role: 'system', content: text(message.content) });
        break;
      case 'user':
        messages.push({ role: 'user', content: userContent(message.content) });
        break;
      case 'assistant': {
        const content: Exclude<AssistantModelMessage['content'], string> = [];
        for (const raw of message.reasoning_details ?? []) {
          const item = raw as Partial<ReasoningDetailItem>;
          if (item.type !== 'reasoning.aero' || item.provider !== model.provider || typeof item.text !== 'string') continue;
          const metadata = replayMetadata(model.provider, item.metadata as Record<string, unknown> | undefined);
          if (Object.keys(metadata).length === 0) continue;
          content.push({ type: 'reasoning', text: item.text, providerOptions: { [model.provider]: metadata } });
        }
        const body = text(message.content);
        if (body) content.push({ type: 'text', text: body });
        for (const call of message.tool_calls ?? []) {
          toolNames.set(call.id, call.function.name);
          content.push({ type: 'tool-call', toolCallId: call.id, toolName: call.function.name, input: parseArguments(call.function.arguments) });
        }
        if (content.length) messages.push({ role: 'assistant', content });
        break;
      }
      case 'tool':
        messages.push({ role: 'tool', content: [{ type: 'tool-result', toolCallId: message.tool_call_id, toolName: toolNames.get(message.tool_call_id) ?? 'unknown', output: { type: 'text', value: text(message.content) } }] });
        break;
    }
  }
  if (messages.length === 0) throw new UnsupportedRequest('messages');
  let tools: ToolSet | undefined;
  if (request.tools?.length) {
    tools = {};
    request.tools.forEach((entry, index) => {
      const last = index === request.tools!.length - 1;
      tools![entry.function.name] = tool({
        description: entry.function.description,
        inputSchema: jsonSchema((entry.function.parameters ?? { type: 'object', properties: {} }) as Parameters<typeof jsonSchema>[0]),
        // The last tool definition closes the cached prefix of the tool list (Anthropic).
        ...(anthropic && last && { providerOptions: { anthropic: EPHEMERAL } }),
      });
    });
  }
  if (anthropic) {
    // Cache breakpoints: the system prompt, the tools (above), the newest message, and the previous request's
    // last message, so the next turn reads everything up to here from the cache.
    const lastSystem = instructions.at(-1);
    if (lastSystem) lastSystem.providerOptions = { anthropic: EPHEMERAL };
    const last = messages.at(-1)!;
    last.providerOptions = { anthropic: EPHEMERAL };
    let previousAssistant = -1;
    messages.forEach((message, index) => { if (message.role === 'assistant') previousAssistant = index; });
    if (previousAssistant > 0) messages[previousAssistant - 1]!.providerOptions = { anthropic: EPHEMERAL };
  }
  const effort = request.reasoning_effort ?? model.defaultEffort;
  if (!model.efforts.includes(effort)) throw new UnsupportedRequest('reasoning_effort');
  const providerOptions: Prepared['providerOptions'] = anthropic
    ? { anthropic: { effort, thinking: { type: 'adaptive', display: 'summarized' }, sendReasoning: true, ...(request.parallel_tool_calls === false && { disableParallelToolUse: true }) } }
    : { openai: { promptCacheKey: cacheKey, reasoningEffort: effort, ...(effort !== 'none' && { reasoningSummary: 'auto' }), store: false, include: ['reasoning.encrypted_content'], ...(request.parallel_tool_calls !== undefined && { parallelToolCalls: request.parallel_tool_calls }) } };
  const choice = request.tool_choice;
  return {
    instructions,
    messages,
    tools,
    toolChoice: !choice ? undefined : typeof choice === 'string' ? choice : { type: 'tool', toolName: choice.function.name },
    providerOptions,
    maxOutputTokens: request.max_completion_tokens ?? request.max_tokens,
    temperature: request.temperature,
    topP: request.top_p,
  };
}
