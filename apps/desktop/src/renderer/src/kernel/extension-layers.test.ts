import { describe, expect, it } from 'vitest';
import type { ExtensionLayerDefinition } from './api';
import { extensionLayerFragment, validateExtensionLayerDefinition } from './extension-layers';

const definition = (patch: Partial<ExtensionLayerDefinition> = {}): ExtensionLayerDefinition => ({
  id: 'demo.layer', label: 'Demo layer', version: 1,
  params: [
    { k: 'amount', label: 'Amount', def: 0.5, min: 0, max: 1 },
    { k: 'tint', label: 'Tint', def: '#FF6600', type: 'color' },
    { k: 'enabled', label: 'Enabled', def: true, type: 'toggle' }
  ],
  renderer: { kind: 'fragment', fragment: 'void main(){ fragColor=vec4(1.0); }' },
  ...patch
});

describe('structured extension layers', () => {
  it('validates a bounded definition and assembles typed uniforms', () => {
    const value = definition();
    expect(() => validateExtensionLayerDefinition(value)).not.toThrow();
    expect(extensionLayerFragment(value, '#version 300 es')).toContain('uniform float u_amount;');
    expect(extensionLayerFragment(value, '#version 300 es')).toContain('uniform vec3 u_tint;');
    expect(extensionLayerFragment(value, '#version 300 es')).toContain('uniform bool u_enabled;');
  });

  it('rejects invalid ids, duplicate parameters, and fragments without main', () => {
    expect(() => validateExtensionLayerDefinition(definition({ id: '../unsafe' }))).toThrow(/invalid id/);
    expect(() => validateExtensionLayerDefinition(definition({ params: [
      { k: 'same', label: 'A', def: 0, min: 0, max: 1 },
      { k: 'same', label: 'B', def: 0, min: 0, max: 1 }
    ] }))).toThrow(/duplicated/);
    expect(() => validateExtensionLayerDefinition(definition({ renderer: { kind: 'fragment', fragment: 'fragColor=vec4(1.0);' } }))).toThrow(/void main/);
  });

  it('requires structured defaults to be safe finite JSON', () => {
    expect(() => validateExtensionLayerDefinition(definition({ defaults: { bad: Number.NaN } }))).toThrow(/must be JSON/);
    expect(() => validateExtensionLayerDefinition(definition({ defaults: { bad: () => true } as any }))).toThrow(/must be JSON/);
    expect(() => validateExtensionLayerDefinition(definition({ defaults: new Date() as any }))).toThrow(/plain objects/);
  });

  it('accepts a declarative mesh renderer and validates its asset field', () => {
    expect(() => validateExtensionLayerDefinition(definition({ renderer: { kind: 'mesh', assetField: 'assetId' } }))).not.toThrow();
    expect(() => validateExtensionLayerDefinition(definition({ renderer: { kind: 'mesh', assetField: '../asset' } }))).toThrow(/assetField/);
  });
});
