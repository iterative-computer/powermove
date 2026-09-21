import type { ImportDefaults } from './api';

export const IMPORT_DEFAULTS_SERVICE = 'media.importDefaults';

export function validatedImportDefaults(value: ImportDefaults): ImportDefaults {
  const x = value?.anchor?.x, y = value?.anchor?.y;
  if (!Number.isFinite(x) || !Number.isFinite(y) || x < 0 || x > 1 || y < 0 || y > 1) {
    throw new Error('Import anchor x and y must be finite numbers between 0 and 1.');
  }
  return { anchor: { x, y } };
}
