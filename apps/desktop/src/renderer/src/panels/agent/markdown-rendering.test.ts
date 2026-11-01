// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import Markdown from './Markdown.svelte';

const openExternal = vi.fn();
vi.mock('../../kernel/bridge', () => ({ bridge: () => ({ openExternal }) }));
let instance: ReturnType<typeof mount>;
let target: HTMLDivElement;
afterEach(() => { if (instance) unmount(instance); target?.remove(); vi.clearAllMocks(); });
function render(text: string, props = {}) {
  target = document.createElement('div'); document.body.append(target);
  instance = mount(Markdown, { target, props: { text, ...props } }); flushSync();
}
it('escapes HTML, keeps unsafe links inert, and loads web images only on request', () => {
  render('<script>alert(1)</script> [unsafe](javascript:alert(1)) ![image](https://example.com/image.png)');
  expect(target.querySelector('script, img, a')).toBeNull();
  expect(target.textContent).toContain('<script>alert(1)</script>');
  const load = target.querySelector<HTMLButtonElement>('.agent-md-media-load')!;
  expect(load.textContent?.trim()).toBe('Show image from example.com');
  load.click(); flushSync();
  expect(target.querySelector('img')?.getAttribute('src')).toBe('https://example.com/image.png');
});
it('renders nested lists, task lists and quotes as real elements', () => {
  render('1. Plan\n   - [x] cut\n   - [ ] grade\n2. Ship\n\n> **Note**\n> - inside');
  expect(target.querySelector('ol')?.getAttribute('start')).toBe('1');
  expect(target.querySelectorAll('ol > li')).toHaveLength(2);
  expect(target.querySelectorAll('ol ul .agent-md-check')).toHaveLength(2);
  expect(target.querySelector('.agent-md-check.is-checked')?.getAttribute('aria-label')).toBe('Done');
  expect(target.querySelector('blockquote ul li')?.textContent?.trim()).toBe('inside');
});
it('puts the streaming caret only on the deepest last paragraph', () => {
  render('- a\n  - b', { streaming: true });
  expect([...target.querySelectorAll('.is-streaming')].map(el => el.textContent)).toEqual(['b']);
});
it('opens safe links through the desktop bridge and preserves italic styling', () => {
  render('*[Guide](https://example.com/guide)*');
  const link = target.querySelector('a')!;
  expect(link.classList.contains('is-italic')).toBe(true);
  link.click();
  expect(openExternal).toHaveBeenCalledWith('https://example.com/guide');
});
it('renders option formatting without nesting interactive links inside buttons', () => {
  render('**[Choose](https://example.com)**', { inline: true, links: false });
  expect(target.querySelector('a, p')).toBeNull();
  expect(target.querySelector('.is-bold')?.textContent).toBe('Choose');
});
it('shows local images, videos and sounds an agent names, under their sentence', () => {
  render('Before: /tmp/claude-501/before-timeline.jpg shows the timeline.\n\n![After drag](</Users/me/Work 2/after.png>)\n\n- Recording: `/tmp/run.mp4`\n- Mix: ~/Desktop/mix.wav');
  const paragraphs = [...target.querySelectorAll('p')].map(el => el.textContent);
  expect(paragraphs).toContain('Before: /tmp/claude-501/before-timeline.jpg shows the timeline.');
  expect(paragraphs.some(text => text?.includes('After drag'))).toBe(false);
  expect([...target.querySelectorAll('img')].map(el => [el.getAttribute('src'), el.alt])).toEqual([
    ['app://powermove/_agent-file?path=%2Ftmp%2Fclaude-501%2Fbefore-timeline.jpg', 'before-timeline.jpg'],
    ['app://powermove/_agent-file?path=%2FUsers%2Fme%2FWork%202%2Fafter.png', 'After drag']
  ]);
  expect(target.querySelector('video')?.getAttribute('src')).toBe('app://powermove/_agent-file?path=%2Ftmp%2Frun.mp4');
  expect(target.querySelector('audio')?.getAttribute('src')).toBe('app://powermove/_agent-file?path=~%2FDesktop%2Fmix.wav');
  const frame = target.querySelector<HTMLButtonElement>('.agent-md-media-frame')!;
  frame.click(); flushSync();
  expect(frame.classList.contains('is-expanded')).toBe(true);
});
it('replaces a file it cannot load with a short note', () => {
  render('![](/tmp/missing.png)');
  target.querySelector('img')!.dispatchEvent(new Event('error')); flushSync();
  expect(target.querySelector('img')).toBeNull();
  expect(target.textContent).toContain("Couldn't show /tmp/missing.png");
});
