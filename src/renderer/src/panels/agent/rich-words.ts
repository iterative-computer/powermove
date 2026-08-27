/* Inline-markdown tokenizer for agent prose.

   Ported from supermove's `toRichWords`. Model output is never rendered as
   HTML — it is split into words carrying two flags: `b` (bold) and `c`
   (inline code). Splitting per WORD rather than per span is what lets the
   reveal animations stagger, and keeps `white-space: pre-wrap` honest. */

export interface RichWord {
  /** The literal text of this word (may be whitespace or a newline). */
  w: string;
  /** Inside `**bold**` or a markdown heading. */
  b: boolean;
  /** Inside a `` `code` `` span. */
  c: boolean;
}

export function toRichWords(text: string): RichWord[] {
  const words: RichWord[] = [];
  for (const rawLine of String(text ?? '').split('\n')) {
    if (words.length > 0) words.push({ w: '\n', b: false, c: false });
    let line = rawLine;
    let lineBold = false;
    const heading = line.match(/^\s*#{1,6}\s+(.*)$/);
    if (heading) {
      line = heading[1] ?? '';
      lineBold = true;
    }
    line = line.replace(/^(\s*)[-*]\s+/, '$1• ');
    // Tokenize **bold** and `code` spans; everything else is plain.
    const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
    for (const part of parts) {
      if (part === '') continue;
      let bold = lineBold;
      let code = false;
      let content = part;
      if (part.startsWith('**') && part.endsWith('**')) {
        bold = true;
        content = part.slice(2, -2);
      } else if (part.startsWith('`') && part.endsWith('`') && part.length > 1) {
        code = true;
        content = part.slice(1, -1);
      }
      for (const piece of content.split(/(\s+)/)) {
        if (piece !== '') words.push({ w: piece, b: bold, c: code });
      }
    }
  }
  return words;
}
