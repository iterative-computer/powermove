const encoder = new TextEncoder();

/** The window's document identity on the engine wire (contract 5). */
export function documentId(id: unknown): string {
  const text = typeof id === 'string' && id ? id : 'untitled';
  return encoder.encode(text).length <= 256 ? text : text.slice(0, 64);
}
