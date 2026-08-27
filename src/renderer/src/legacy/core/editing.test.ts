import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './editing';

describe('legacy editing install', () => {
  it('installs the frozen source-edit vocabulary', () => {
    const PM: PMRegistry = {};
    install(PM);

    expect(PM.Edit.operations.set_property).toEqual({
      target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'],
    });
    expect(PM.Edit.operations.set_content).toEqual({ target: 'layer', fields: ['patch'] });
    expect(PM.Edit.operations.transform_layers).toEqual({
      target: 'layer-collection', fields: ['transform', 'state'],
    });
    expect(PM.Edit.operations.set_transition).toEqual({
      target: 'layer', fields: ['layer', 'edge', 'transition'],
    });
    expect(Object.keys(PM.Edit.operations)).toHaveLength(19);
  });
});
