export interface CompatibleProviderConfig {
  baseUrl: string;
  model: string;
  vision: boolean;
  hasKey: boolean;
  /** Advertised model IDs; older connections fall back to the configured model. */
  models?: string[];
  /** Something about the connection that will hurt results, such as a local
   * server running the model with too little context. */
  warning?: string;
}
export interface CompatibleProviderInput {
  baseUrl: string;
  model: string;
  vision: boolean;
  apiKey?: string;
}
/** What a provider says about one model, when it says anything. */
export interface CompatibleModelInfo {
  id: string;
  /** The model can read images. Unknown when the provider does not say. */
  vision?: boolean;
}
export interface CompatibleModelList {
  models: CompatibleModelInfo[];
  /** Models left out because they cannot call tools, which the agent needs. */
  hidden: number;
  /** A key is saved for this address, so the key field may stay blank. */
  hasKey: boolean;
}
/** A server answering on this Mac at one of the local presets' addresses. */
export interface CompatibleLocalServer {
  preset: string;
  baseUrl: string;
  models: number;
}
export const DEFAULT_COMPATIBLE_PROVIDER: CompatibleProviderConfig = {
  baseUrl: 'http://localhost:11434/v1', model: '', vision: false, hasKey: false,
};

export interface CompatiblePreset {
  id: string;
  name: string;
  /** Empty for "Other", where the person types the address. */
  baseUrl: string;
  kind: 'local' | 'cloud' | 'other';
  /** Where to create a key, for cloud providers. */
  keyUrl?: string;
  /** How this provider's keys begin, shown as the field's placeholder. */
  keyPrefix?: string;
  /** Where to get the app, for local servers. */
  downloadUrl?: string;
}

/* Addresses are each provider's documented OpenAI-compatible base URL. */
export const COMPATIBLE_PRESETS: readonly CompatiblePreset[] = [
  { id: 'ollama', name: 'Ollama', baseUrl: 'http://localhost:11434/v1', kind: 'local', downloadUrl: 'https://ollama.com/download' },
  { id: 'lmstudio', name: 'LM Studio', baseUrl: 'http://localhost:1234/v1', kind: 'local', downloadUrl: 'https://lmstudio.ai' },
  { id: 'llamacpp', name: 'llama.cpp server', baseUrl: 'http://localhost:8080/v1', kind: 'local', downloadUrl: 'https://github.com/ggml-org/llama.cpp' },
  { id: 'jan', name: 'Jan', baseUrl: 'http://localhost:1337/v1', kind: 'local', downloadUrl: 'https://jan.ai' },
  { id: 'openrouter', name: 'OpenRouter', baseUrl: 'https://openrouter.ai/api/v1', kind: 'cloud', keyUrl: 'https://openrouter.ai/settings/keys', keyPrefix: 'sk-or-' },
  { id: 'openai', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', kind: 'cloud', keyUrl: 'https://platform.openai.com/api-keys', keyPrefix: 'sk-' },
  { id: 'anthropic', name: 'Anthropic', baseUrl: 'https://api.anthropic.com/v1', kind: 'cloud', keyUrl: 'https://console.anthropic.com/settings/keys', keyPrefix: 'sk-ant-' },
  { id: 'gemini', name: 'Google Gemini', baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai', kind: 'cloud', keyUrl: 'https://aistudio.google.com/apikey', keyPrefix: 'AIza' },
  { id: 'groq', name: 'Groq', baseUrl: 'https://api.groq.com/openai/v1', kind: 'cloud', keyUrl: 'https://console.groq.com/keys', keyPrefix: 'gsk_' },
  { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', kind: 'cloud', keyUrl: 'https://platform.deepseek.com/api_keys', keyPrefix: 'sk-' },
  { id: 'xai', name: 'xAI', baseUrl: 'https://api.x.ai/v1', kind: 'cloud', keyUrl: 'https://console.x.ai', keyPrefix: 'xai-' },
  { id: 'mistral', name: 'Mistral', baseUrl: 'https://api.mistral.ai/v1', kind: 'cloud', keyUrl: 'https://console.mistral.ai/api-keys' },
  { id: 'other', name: 'Other compatible service', baseUrl: '', kind: 'other' }
];

/** The preset an address belongs to, or "Other". */
export function presetFor(baseUrl: string): CompatiblePreset {
  let normalized = '';
  try { normalized = providerUrl(baseUrl); } catch { /* an unfinished address is Other */ }
  return COMPATIBLE_PRESETS.find(preset => preset.baseUrl && preset.baseUrl === normalized)
    ?? COMPATIBLE_PRESETS.at(-1)!;
}

export function isLocalAddress(value: string): boolean {
  try { return ['localhost', '127.0.0.1', '[::1]'].includes(new URL(value).hostname); } catch { return false; }
}

export function providerUrl(value: string): string {
  // A bare host such as localhost:11434 is what people copy from a server log.
  const text = value.trim();
  const url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text
    : /^(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/i.test(text) ? `http://${text}` : `https://${text}`);
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (url.username || url.password || url.search || url.hash || !(url.protocol === 'https:' || local && url.protocol === 'http:')) {
    throw new Error('Use an HTTPS API address, or HTTP for a local model on this Mac.');
  }
  // Gateways such as Sub2API commonly provide the server origin as their
  // base URL. Accept that and a pasted Chat Completions endpoint as well as
  // the explicit /v1 base, while preserving deployment prefixes.
  let pathname = url.pathname.replace(/\/+$/, '').replace(/\/chat\/completions$/, '');
  if (!pathname) pathname = '/v1';
  url.pathname = pathname;
  return url.href.replace(/\/+$/, '');
}
