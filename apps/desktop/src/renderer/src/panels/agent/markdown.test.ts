import { describe, expect, it } from 'vitest';

import { blocksFromMarkdown, inlineRuns, plainText, wordsFromRuns } from './markdown';

describe('inlineRuns', () => {
  it('parses code, bold, italic, and links without leaking markers', () => {
    expect(inlineRuns('run `npm test` then **stop**')).toEqual([
      { text: 'run ' }, { text: 'npm test', c: true }, { text: ' then ' }, { text: 'stop', b: true }
    ]);
    expect(inlineRuns('see [the docs](https://example.com/docs) now')).toEqual([
      { text: 'see ' }, { text: 'the docs', href: 'https://example.com/docs' }, { text: ' now' }
    ]);
    expect(inlineRuns('*soft* and _quiet_')).toEqual([
      { text: 'soft', i: true }, { text: ' and ' }, { text: 'quiet', i: true }
    ]);
  });

  it('keeps snake_case words and unclosed markers as literal text', () => {
    expect(inlineRuns('use file_path here')).toEqual([{ text: 'use file_path here' }]);
    expect(inlineRuns('this is **not closed')).toEqual([{ text: 'this is **not closed' }]);
  });

  it('links bare URLs and drops unsafe link targets', () => {
    expect(inlineRuns('Portrait: https://commons.wikimedia.org/wiki/File:Tim.jpg, CC BY')).toEqual([
      { text: 'Portrait: ' },
      { text: 'https://commons.wikimedia.org/wiki/File:Tim.jpg', href: 'https://commons.wikimedia.org/wiki/File:Tim.jpg' },
      { text: ', CC BY' }
    ]);
    expect(inlineRuns('[x](javascript:alert(1))').some((run) => run.href)).toBe(false);
    const wiki = 'https://commons.wikimedia.org/wiki/File:Tim_Cook_March_2026_(cropped).jpg';
    expect(inlineRuns(`[Commons](${wiki}), CC BY`)).toEqual([{ text: 'Commons', href: wiki }, { text: ', CC BY' }]);
    expect(inlineRuns(`See ${wiki}.`)).toEqual([{ text: 'See ' }, { text: wiki, href: wiki }, { text: '.' }]);
  });

  it('links autolinks, titled links, and images without stray markers', () => {
    expect(inlineRuns('Docs: <https://x.y/a>.')).toEqual([{ text: 'Docs: ' }, { text: 'https://x.y/a', href: 'https://x.y/a' }, { text: '.' }]);
    expect(inlineRuns('[Guide](https://x.y/g "The guide")')).toEqual([{ text: 'Guide', href: 'https://x.y/g' }]);
    expect(inlineRuns('![Frame 12](https://x.y/f.png)')).toEqual([{ text: '', media: { kind: 'image', src: 'https://x.y/f.png', path: 'https://x.y/f.png', alt: 'Frame 12', remote: true } }]);
    expect(inlineRuns('![](http://x.y/shots/f.png)')).toEqual([{ text: 'f.png', href: 'http://x.y/shots/f.png' }]);
    // Targets main will not open (mail, local files) stay readable text.
    expect(inlineRuns('[mail](mailto:a@b.c)')).toEqual([{ text: 'mail' }]);
    expect(inlineRuns('[app.ts](/Users/me/app.ts:12)')).toEqual([{ text: 'app.ts' }]);
  });
});

describe('blocksFromMarkdown', () => {
  it('splits paragraphs, headings, lists, quotes, and fences', () => {
    const blocks = blocksFromMarkdown([
      '## Plan', '', 'First line', 'same paragraph', '', '- one', '- two', '1. first', '', '> aside', '', '```ts', 'const a = 1;', '```'
    ].join('\n'));
    expect(blocks.map((block) => block.kind)).toEqual(['h', 'p', 'list', 'list', 'quote', 'code']);
    expect(plainText(blocks)).toBe('Plan\nFirst line same paragraph\none\ntwo\nfirst\naside\nconst a = 1;');
    expect(blocks[3]).toMatchObject({ kind: 'list', ordered: true, start: 1 });
    expect(blocks[5]).toMatchObject({ kind: 'code', lang: 'ts' });
  });

  it('renders a still-open fence as code so streaming does not flicker', () => {
    const blocks = blocksFromMarkdown('```\nhalf');
    expect(blocks).toEqual([{ kind: 'code', text: 'half' }]);
  });

  it('is safe on empty input', () => {
    expect(blocksFromMarkdown('')).toEqual([]);
    expect(blocksFromMarkdown(undefined as any)).toEqual([]);
  });
});

describe('wordsFromRuns', () => {
  it('splits runs into words with stable keys and keeps links whole', () => {
    const words = wordsFromRuns([{ text: 'a b' }, { text: 'x y', href: 'https://x.y' }]);
    expect(words.map((word) => [word.key, word.text])).toEqual([['0-0', 'a'], ['0-1', ' '], ['0-2', 'b'], ['1', 'x y']]);
  });
});

 it('keeps inline code in one element and handles escaped delimiters', () => {
   expect(wordsFromRuns(inlineRuns('`bun run test`'))).toEqual([{text: 'bun run test', c: true, key: '0'}]);
   expect(inlineRuns('``a `tick` here``')).toEqual([{text: 'a `tick` here', c: true}]);
   expect(inlineRuns(String.raw`\*literal\*`)).toEqual([{text: '*literal*'}]);
 });

it('renders aligned tables with escaped pipes and ragged rows', () => {
  const blocks = blocksFromMarkdown('| Name | Value |\n| :--- | ---: |\n| **Clip** | `ready` |\n| a\\|b |\n\nAfter');
  expect(blocks[0]).toEqual({ kind: 'table', headers: [[{ text: 'Name' }], [{ text: 'Value' }]],
    align: ['left', 'right'], rows: [[[{ text: 'Clip', b: true }], [{ text: 'ready', c: true }]], [[{ text: 'a|b' }], []]] });
  expect(plainText(blocks)).toBe('Name | Value\nClip | ready\na|b | \nAfter');
  expect(blocksFromMarkdown('a | b\nnot | a table').map(block => block.kind)).toEqual(['p']);
});
it('preserves dividers and formats strikethrough', () => {
  expect(blocksFromMarkdown('Before\n\n---\n\nAfter').map(block => block.kind)).toEqual(['p', 'hr', 'p']);
  expect(inlineRuns('~~old~~ and ~~**bold**~~')).toEqual([{ text: 'old', s: true }, { text: ' and ' }, { text: 'bold', b: true, s: true }]);
});

describe('full markdown', () => {
  const tree = (text: string) => JSON.parse(JSON.stringify(blocksFromMarkdown(text)));

  it('nests lists by indentation and keeps multi-paragraph items together', () => {
    const [list] = tree('- one\n  - inner\n    1. deep\n- two\n\n  more of two\n- three\nlazy line');
    expect(list.items).toHaveLength(3);
    expect(list.items[0].blocks[1]).toMatchObject({ kind: 'list', items: [{ blocks: [{ kind: 'p' }, { kind: 'list', ordered: true }] }] });
    expect(list.items[1].blocks.map((block: { kind: string }) => block.kind)).toEqual(['p', 'p']);
    expect(plainText(list.items[2].blocks)).toBe('three lazy line');
  });

  it('reads task lists, start numbers, ) delimiters and code inside items', () => {
    const [tasks] = tree('- [x] shipped\n- [ ] pending');
    expect(tasks.items.map((item: { checked: boolean }) => item.checked)).toEqual([true, false]);
    expect(plainText(tasks.items[0].blocks)).toBe('shipped');
    expect(tree('3) c\n4) d')[0]).toMatchObject({ ordered: true, start: 3, items: [{}, {}] });
    const [withCode] = tree('1. run:\n   ```sh\n   bun test\n   ```\n2. done');
    expect(withCode.items[0].blocks[1]).toEqual({ kind: 'code', text: 'bun test', lang: 'sh' });
    expect(withCode.items).toHaveLength(2);
  });

  it('nests quotes with any block inside', () => {
    const [quote] = tree('> # Note\n> - a\n>\n> > deeper\n> lazy');
    expect(quote.blocks.map((block: { kind: string }) => block.kind)).toEqual(['h', 'list', 'quote']);
    expect(plainText(quote.blocks[2].blocks)).toBe('deeper lazy');
  });

  it('handles underlined headings, hard breaks, longer fences and bold italic', () => {
    expect(tree('Title\n=====\n\nSub\n---')).toMatchObject([{ kind: 'h', level: 1 }, { kind: 'h', level: 2 }]);
    expect(plainText(blocksFromMarkdown('a  \nb\\\nc\nd<br>e'))).toBe('a\nb\nc d\ne');
    expect(tree('````md\n```\ninner\n```\n````')[0]).toEqual({ kind: 'code', text: '```\ninner\n```', lang: 'md' });
    expect(inlineRuns('***both*** __bold__')).toEqual([{ text: 'both', b: true, i: true }, { text: ' ' }, { text: 'bold', b: true }]);
  });

  it('keeps simple inline HTML meaning without rendering HTML', () => {
    expect(inlineRuns('<kbd>⌘K</kbd> <b>bold</b> <img src="/tmp/a.png" alt="A">')).toEqual([
      { text: '⌘K', c: true }, { text: ' ' }, { text: 'bold', b: true }, { text: ' ' },
      { text: '', media: { kind: 'image', src: 'app://powermove/_agent-file?path=%2Ftmp%2Fa.png', path: '/tmp/a.png', alt: 'A' } }
    ]);
    expect(inlineRuns('<script>x</script>')).toEqual([{ text: '<script>x</script>' }]);
  });

  it('keeps a linked image as both picture and link', () => {
    expect(inlineRuns('[![shot](/tmp/s.png)](https://x.y)')).toEqual([
      { text: 'shot', href: 'https://x.y' },
      { text: '', media: { kind: 'image', src: 'app://powermove/_agent-file?path=%2Ftmp%2Fs.png', path: '/tmp/s.png', alt: 'shot' } }
    ]);
  });

  it('streams a list item and a fence without breaking earlier blocks', () => {
    expect(tree('Intro\n-').map((block: { kind: string }) => block.kind)).toEqual(['p']);
    expect(tree('Intro\n- a').map((block: { kind: string }) => block.kind)).toEqual(['p', 'list']);
    expect(tree('- a\n  ```\n  half')[0].items[0].blocks[1]).toEqual({ kind: 'code', text: 'half' });
  });
});

describe('local media', () => {
  it('turns local image markdown, bare paths and code paths into media markers', () => {
    const [shot] = blocksFromMarkdown('![shot](/tmp/a.png)') as Array<{ runs: unknown[]; media?: Array<{ kind: string; path: string }> }>;
    expect(shot!.runs).toEqual([]);
    expect(shot!.media).toEqual([{ kind: 'image', path: '/tmp/a.png', alt: 'shot', src: 'app://powermove/_agent-file?path=%2Ftmp%2Fa.png' }]);
    expect(inlineRuns('see /tmp/b.gif.').map(run => run.media?.path ?? run.text)).toEqual(['see /tmp/b.gif', '/tmp/b.gif', '.']);
    expect(inlineRuns('`/tmp/c.mov`').map(run => run.media?.kind ?? run.text)).toEqual(['/tmp/c.mov', 'video']);
    expect(inlineRuns('file:///tmp/d.webp')[0]).toEqual({ text: 'file:///tmp/d.webp' });
    expect(inlineRuns('![d](file:///tmp/d%20e.webp)')[0]!.media!.path).toBe('/tmp/d e.webp');
  });

  it('leaves non-media paths, relative paths, traversal and remote images alone', () => {
    for (const text of ['/etc/passwd', 'shots/a.png', '![x](/tmp/../etc/a.png)', 'a/tmp/x.png', '/tmp/a.pngx']) {
      expect(inlineRuns(text).some(run => run.media), text).toBe(false);
    }
    expect(inlineRuns('see https://x.y/r.png')).toEqual([{ text: 'see ' }, { text: 'https://x.y/r.png', href: 'https://x.y/r.png' }]);
  });

  it('shows each file once per block and keeps it out of the reveal words', () => {
    const [block] = blocksFromMarkdown('/tmp/a.png then `/tmp/a.png`') as Array<{ runs: Parameters<typeof wordsFromRuns>[0]; media?: unknown[] }>;
    expect(block!.media).toHaveLength(1);
    expect(wordsFromRuns(inlineRuns('/tmp/a.png')).map(word => word.text)).toEqual(['/tmp/a.png']);
  });
});

