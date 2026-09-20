import { expect, it } from 'vitest';
import { EDITOR_HELPER_EXPORTS } from '../../../shared/extension-runtime';
import * as helpers from './editor-helpers';

it('exposes the same public helpers to compiled extensions and built-ins', () => {
  expect([...EDITOR_HELPER_EXPORTS].sort()).toEqual(Object.keys(helpers).sort());
});
