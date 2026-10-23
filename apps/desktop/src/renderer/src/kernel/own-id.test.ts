import { expect, it } from 'vitest';
import { ownId, qualifyId } from './sandbox-host';

// A sandboxed extension owns the bare extension id and anything under `<id>.`.
// The bare id is unique so it can't impersonate another owner; the dot is still
// required for suffixes so `foo` can't claim a sibling `foo-bar`'s namespace.
it('accepts the bare extension id and its dot-namespace, rejects siblings/foreign', () => {
  expect(ownId('private-extension-09', 'private-extension-09')).toBe(true);
  expect(ownId('private-extension-09', 'private-extension-09.panel')).toBe(true);
  expect(ownId('private-extension-09', 'private-extension-09-extra')).toBe(false);
  expect(ownId('foo', 'foo-bar.command')).toBe(false);
  expect(ownId('foo', 'bar')).toBe(false);
});

// The system relocates any id into the extension's namespace (idempotent), so authors
// use bare ids and nothing can land outside their own namespace.
it('qualifies bare and foreign ids into the extension namespace, leaves own ids alone', () => {
  expect(qualifyId('private-extension-09', 'hud')).toBe('private-extension-09.hud');
  expect(qualifyId('private-extension-09', 'private-extension-09')).toBe('private-extension-09');
  expect(qualifyId('private-extension-09', 'private-extension-09.panel')).toBe('private-extension-09.panel');
  expect(qualifyId('private-extension-09', 'blur')).toBe('private-extension-09.blur');
  expect(qualifyId('foo', 'foo-bar.command')).toBe('foo.foo-bar.command');
});
