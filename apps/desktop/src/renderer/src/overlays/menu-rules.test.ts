import { describe, expect, it } from 'vitest';

import { tidyMenu } from './menu-rules';

describe('tidyMenu', () => {
  it('keeps separators only between groups and drops titles with nothing under them', () => {
    expect(tidyMenu(['-', { header: 'Empty' }, '-', '-', { label: 'A' }, '-', '-', { header: 'B' }, { label: 'B1' }, { header: 'Dangling' }, '-'])).toEqual([
      { label: 'A' }, '-', { header: 'B' }, { label: 'B1' }
    ]);
  });

  it('keeps an icon only on the first row that uses it', () => {
    expect(tidyMenu([{ label: 'Start', icon: 'frame' }, { label: 'End', icon: 'frame' }, { label: 'Odd' }, '-', { label: 'Lock', icon: 'lock' }, { label: 'Frame', icon: 'frame' }])).toEqual([
      { label: 'Start', icon: 'frame' }, { label: 'End', icon: undefined }, { label: 'Odd' }, '-', { label: 'Lock', icon: 'lock' }, { label: 'Frame', icon: undefined }
    ]);
  });
});
