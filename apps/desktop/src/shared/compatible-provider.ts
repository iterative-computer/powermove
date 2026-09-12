export interface CompatibleProviderConfig {
  baseUrl: string;
  model: string;
  vision: boolean;
  hasKey: boolean;
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
  return url.href.replace(/\/+$/, '');
}
