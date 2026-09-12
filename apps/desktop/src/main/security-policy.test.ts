import { describe, expect, it } from 'vitest';

import { CONTENT_SECURITY_POLICY, SANDBOX_CONTENT_SECURITY_POLICY } from './security-policy';

describe('renderer security policy', () => {
  it('allows trusted extensions to make HTTP requests and WebSocket connections', () => {
    const connect = CONTENT_SECURITY_POLICY.split(';').map(value => value.trim())
      .find(value => value.startsWith('connect-src '))?.split(/\s+/).slice(1);
    expect(connect).toEqual(["'self'", 'blob:', 'http:', 'https:', 'ws:', 'wss:']);
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("connect-src 'none'");
  });

  it('forbids eval in the privileged editor and confines it to the generated-script sandbox', () => {
    expect(CONTENT_SECURITY_POLICY).toContain("script-src 'self'");
    expect(CONTENT_SECURITY_POLICY).not.toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("connect-src 'none'");
  });
});
