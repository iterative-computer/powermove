/* CSS font stacks for previewing a family by name.

   Chromium cannot resolve Apple's system families ("SF Pro Display",
   "SF Pro Text", "SF Mono", "New York") through a plain font-family name;
   only the generic aliases (-apple-system, ui-monospace, ui-serif) reach
   them. Without this mapping a preview labelled SF Pro renders in the
   default serif. Every stack ends in a sane generic so a missing font
   degrades to the UI face rather than Times. */

const escape = (value: string) => value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');

const APPLE_SYSTEM: Record<string, string> = {
  'sf pro': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf pro display': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf pro text': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf pro rounded': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf compact': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf compact display': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf compact text': '-apple-system, BlinkMacSystemFont, system-ui',
  'sf mono': 'ui-monospace, Menlo, monospace',
  'new york': 'ui-serif, Georgia, serif',
};

export function cssFontStack(family: string | null | undefined): string {
  const name = String(family ?? '').trim();
  if (!name) return 'inherit';
  const system = APPLE_SYSTEM[name.toLowerCase()];
  const quoted = `"${escape(name)}"`;
  if (system) return `${quoted}, ${system}`;
  const generic = /mono|code|courier|consolas|menlo/i.test(name) ? 'ui-monospace, monospace'
    : /serif|georgia|times|garamond|baskerville|playfair|caslon|didot|bodoni|new york/i.test(name) && !/sans/i.test(name) ? 'ui-serif, serif'
    : 'system-ui, sans-serif';
  return `${quoted}, ${generic}`;
}
