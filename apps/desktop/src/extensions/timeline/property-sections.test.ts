import { describe, expect, it } from 'vitest';
import { propertySection, sectionedPropertyRows } from './property-sections';

const label = (fx: any) => fx.type === 'gaussianBlur' ? 'Gaussian Blur' : fx.type;

describe('timeline property sections', () => {
  const layer = { id: 'text', fx: [{ id: 'fx1', type: 'gaussianBlur' }, { id: 'fx2', type: 'gaussianBlur' }] };

  it('names each property after its Properties panel section', () => {
    expect(propertySection(layer, { key: 'c.size', group: 'Content' }, label).label).toBe('Content');
    expect(propertySection(layer, { key: 'opacity', group: 'Transform' }, label).label).toBe('Transform');
    expect(propertySection(layer, { key: 'scale' }, label).label).toBe('Transform');
    expect(propertySection(layer, { key: 'fx1.amount', group: 'gaussianBlur' }, label).label).toBe('Gaussian Blur');
    expect(propertySection(layer, { key: 'm.a.feather', group: 'Mask 1' }, label).label).toBe('Mask 1');
    expect(propertySection(layer, { key: 'l.blend', group: 'Layer' }, label).label).toBe('Layer');
  });

  it('orders sections like the Properties panel and heads each one once', () => {
    const rows = sectionedPropertyRows(layer, [
      { key: 'position.x', label: 'Position X', group: 'Transform' },
      { key: 'fx2.amount', label: 'amount', group: 'gaussianBlur' },
      { key: 'c.size', label: 'Size', group: 'Content' },
      { key: 'opacity', label: 'Opacity', group: 'Transform' },
      { key: 'fx1.amount', label: 'amount', group: 'gaussianBlur' },
    ], label);
    expect(rows.map(row => row.kind === 'heading' ? `# ${row.label}` : row.key)).toEqual([
      '# Content', 'c.size',
      '# Transform', 'position.x', 'opacity',
      '# Gaussian Blur', 'fx1.amount',
      '# Gaussian Blur', 'fx2.amount',
    ]);
    expect(rows.every(row => row.L === layer)).toBe(true);
  });
});
