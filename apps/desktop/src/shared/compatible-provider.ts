export interface CompatibleProviderConfig {
  baseUrl: string;
  model: string;
  vision: boolean;
  hasKey: boolean;
  /** Advertised model IDs; older connections fall back to the configured model. */
  models?: string[];
}
export interface CompatibleProviderInput {
  baseUrl: string;
  model: string;
  vision: boolean;
  apiKey?: string;
}
export const DEFAULT_COMPATIBLE_PROVIDER: CompatibleProviderConfig = {
  baseUrl: 'http://localhost:11434/v1', model: '', vision: false, hasKey: false,
};

export function providerUrl(value: string): string {
  const url = new URL(value);
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
