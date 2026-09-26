/** A hint earns its line only when it says something the label does not. */
export function usefulHint(label: string, hint: string | undefined): string | null {
  const text = hint?.trim();
  if (!text) return null;
  const plain = (value: string) => value.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();
  const a = plain(label);
  const b = plain(text);
  return !b || a === b || a.includes(b) ? null : text;
}
