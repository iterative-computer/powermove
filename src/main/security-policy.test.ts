import { describe, expect, it } from 'vitest';

import { CONTENT_SECURITY_POLICY, SANDBOX_CONTENT_SECURITY_POLICY } from './security-policy';

describe('renderer security policy', () => {
  it('forbids eval in the privileged editor and confines it to the generated-script sandbox', () => {
    expect(CONTENT_SECURITY_POLICY).toContain("script-src 'self'");
    expect(CONTENT_SECURITY_POLICY).not.toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("connect-src 'none'");
  });
});
