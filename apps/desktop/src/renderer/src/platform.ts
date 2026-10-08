export function isWindows(platform = typeof navigator === 'undefined' ? '' : navigator.platform): boolean {
  return /^Win/i.test(platform);
}

/** Existing extensions use Mac glyphs; render the equivalent Windows keys. */
export function shortcutLabel(label: string, platform?: string): string {
  if (!isWindows(platform)) return label;
  return label.replace(/[⌘⌃⇧⌥]+/g, glyphs => [/[⌘⌃]/.test(glyphs) ? 'Ctrl' : '', glyphs.includes('⇧') ? 'Shift' : '', glyphs.includes('⌥') ? 'Alt' : ''].filter(Boolean).join('+') + '+').replace(/↵/g, 'Enter').replace(/\+\+/g, '+');
}
