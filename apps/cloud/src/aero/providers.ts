/* Where Aero's provider calls go, and the one call besides generation: token counting (free, no generation). */
import type { LanguageModel } from 'ai';
import { createAnthropic } from '@ai-sdk/anthropic';
import { createOpenAI } from '@ai-sdk/openai';
import type { AeroModel, Provider } from './catalogue';
import { V2Error } from '../v2-error';

export type Fetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]']);
/** A development base URL (the AI simulator). Only loopback hosts, so a provider key can't leave for anywhere else. */
function localBase(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const url = new URL(raw);
  if (!LOCAL_HOSTS.has(url.hostname)) throw new V2Error(503, 'inference_unavailable');
  return raw.replace(/\/$/, '');
}

export interface Endpoint { base: string; headers: Record<string, string> }
/** The provider's API base and the headers our own (non-SDK) calls to it carry. */
export function endpointFor(env: CloudflareBindings, provider: Provider): Endpoint {
  if (provider === 'openai') {
    if (!env.OPENAI_API_KEY) throw new V2Error(503, 'inference_unavailable');
    return { base: localBase(env.AERO_OPENAI_BASE_URL) ?? 'https://api.openai.com/v1', headers: { authorization: `Bearer ${env.OPENAI_API_KEY}`, 'content-type': 'application/json' } };
  }
  if (!env.ANTHROPIC_API_KEY) throw new V2Error(503, 'inference_unavailable');
  return { base: localBase(env.AERO_ANTHROPIC_BASE_URL) ?? 'https://api.anthropic.com/v1', headers: { 'x-api-key': env.ANTHROPIC_API_KEY, 'anthropic-version': '2023-06-01', 'content-type': 'application/json' } };
}

/** The provider model, every call through `fetch` (the meter's: it reserves, caps and records the request). */
export function languageModel(env: CloudflareBindings, model: AeroModel, fetch: Fetch): LanguageModel {
  const { base } = endpointFor(env, model.provider);
  if (model.provider === 'openai') return createOpenAI({ apiKey: env.OPENAI_API_KEY, baseURL: base, fetch: fetch as typeof globalThis.fetch }).responses(model.providerModel);
  return createAnthropic({ apiKey: env.ANTHROPIC_API_KEY, baseURL: base, fetch: fetch as typeof globalThis.fetch })(model.providerModel);
}

const COUNT_FIELDS: Record<Provider, readonly string[]> = {
  anthropic: ['model', 'system', 'messages', 'tools', 'tool_choice', 'thinking'],
  openai: ['model', 'instructions', 'input', 'tools', 'tool_choice', 'parallel_tool_calls', 'reasoning', 'text'],
};

/** The provider's own count of a generation request's input tokens. Throws when it can't say. */
export async function countInput(endpoint: Endpoint, provider: Provider, body: Record<string, unknown>, fetch: Fetch): Promise<number> {
  const subset = Object.fromEntries(COUNT_FIELDS[provider].filter(field => body[field] !== undefined).map(field => [field, body[field]]));
  const url = provider === 'anthropic' ? `${endpoint.base}/messages/count_tokens` : `${endpoint.base}/responses/input_tokens`;
  const response = await fetch(url, { method: 'POST', headers: endpoint.headers, body: JSON.stringify(subset), signal: AbortSignal.timeout(10_000) });
  if (!response.ok) throw new Error(`token count answered ${response.status}`);
  const counted = (await response.json() as { input_tokens?: unknown }).input_tokens;
  if (typeof counted !== 'number' || !Number.isInteger(counted) || counted < 0) throw new Error('token count unreadable');
  return counted;
}
