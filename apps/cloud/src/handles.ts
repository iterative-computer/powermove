export const HANDLE_RE = /^[a-z0-9][a-z0-9-]{1,38}$/;
const RESERVED = new Set(['powermove','admin','store','api','git','auth','me','settings','help','support','www','cloud','team','official','google','github','apple','microsoft','openai','anthropic','meta','amazon','nvidia','adobe','figma']);
export function isReserved(handle: string): boolean { return handle.startsWith('powermove-') || RESERVED.has(handle); }
