import { describe, expect, it } from 'vitest';
import { createServicesRegistry } from './services';

describe('services registry', () => {
  it('registers and retrieves a typed implementation', () => {
    const services = createServicesRegistry();
    const timeline = { frameView: () => 'framed' };

    expect(services.get<typeof timeline>('timeline')).toBeNull();
    services.register('timeline', timeline);
    expect(services.get<typeof timeline>('timeline')).toBe(timeline);
  });

  it('replaces by name and disposal restores the previous implementation', () => {
    const services = createServicesRegistry();
    const first = { version: 1 };
    const second = { version: 2 };
    const firstHandle = services.register('viewer', first);
    const secondHandle = services.register('viewer', second);

    expect(services.get('viewer')).toBe(second);
    secondHandle.dispose();
    expect(services.get('viewer')).toBe(first);
    firstHandle.dispose();
    expect(services.get('viewer')).toBeNull();
  });

  it('supports idempotent and out-of-order disposal', () => {
    const services = createServicesRegistry();
    const first = services.register('inspector', { version: 1 });
    const second = services.register('inspector', { version: 2 });

    first.dispose();
    first.dispose();
    expect(services.get<{ version: number }>('inspector')).toEqual({ version: 2 });
    second.dispose();
    expect(services.get('inspector')).toBeNull();
  });

  it('clears every service', () => {
    const services = createServicesRegistry();
    services.register('timeline', {});
    services.register('viewer', {});
    services.clear();
    expect(services.get('timeline')).toBeNull();
    expect(services.get('viewer')).toBeNull();
  });
});
