// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { pasteMedia } from './media-paste';

afterEach(() => document.body.replaceChildren());

function harness(files: File[], target: HTMLElement = document.body, items?: any[]) {
  const PM = { proj: { id: 'project' }, time: 3, importFiles: vi.fn(async (_files: File[], _options: unknown) => {}) };
  const event = new ClipboardEvent('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', { value: { files, items } });
  vi.spyOn(event, 'composedPath').mockReturnValue([target, document.body, window]);
  return { PM, event };
}

it('imports copied images, audio and video together at the original playhead', () => {
  const files = [new File(['image'], 'image.png', { type: 'image/png' }),
    new File(['audio'], 'sound.wav', { type: 'audio/wav' }), new File(['video'], 'clip.mp4')];
  const { PM, event } = harness(files);
  expect(pasteMedia(PM, event)).toBe(true);
  expect(event.defaultPrevented).toBe(true);
  expect(PM.importFiles).toHaveBeenCalledExactlyOnceWith(files, { project: PM.proj, placement: { at: 3 }, sequence: false });
});

it('accepts file items when a provider omits the file list, without double importing', () => {
  const file = new File(['image'], 'image.png');
  const items = [{ kind: 'file', getAsFile: () => file }, { kind: 'file', getAsFile: () => null }];
  const { PM, event } = harness([], document.body, items);
  expect(pasteMedia(PM, event)).toBe(true);
  expect(PM.importFiles.mock.calls[0]?.[0]).toEqual([file]);
});

it.each(['input', 'textarea', 'editable', 'transcript'])('keeps media paste local to a %s', kind => {
  const field = document.createElement(kind === 'input' || kind === 'textarea' ? kind : 'div');
  if (kind === 'editable') field.contentEditable = 'true';
  if (kind === 'transcript') field.dataset.nativeText = '';
  const inner = document.createElement('span'); field.append(inner); document.body.append(field);
  const { PM, event } = harness([new File(['image'], 'image.png')], inner);
  expect(pasteMedia(PM, event)).toBe(false);
  expect(event.defaultPrevented).toBe(false);
  expect(PM.importFiles).not.toHaveBeenCalled();
});

it('leaves plain text and handled events alone', () => {
  const { PM, event } = harness([]);
  expect(pasteMedia(PM, event)).toBe(false);
  const handled = harness([new File(['image'], 'image.png')]); handled.event.preventDefault();
  expect(pasteMedia(handled.PM, handled.event)).toBe(false);
  expect(handled.PM.importFiles).not.toHaveBeenCalled();
});
