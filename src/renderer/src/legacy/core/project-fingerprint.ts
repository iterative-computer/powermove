/** Stable across reopen and independent of playhead, selection, and panel layout. */
export async function projectFingerprint(projectJSON: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(projectJSON));
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
}
