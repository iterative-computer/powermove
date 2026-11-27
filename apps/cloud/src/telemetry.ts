/* Server-side PostHog events (the desktop app's project, apps/native/crates/telemetry): best-effort, never blocking
   or failing the request that sends them. Usage and billing facts only, never prompt or response content.
   On only when POSTHOG_HOST and POSTHOG_KEY are set and the Worker is production (local stacks set
   POSTHOG_LOCAL=1 to send). */

export type Emit = (event: string, distinctId: string, properties?: Record<string, unknown>) => void;

/** Whether this environment sends events. */
export function telemetryOn(env: CloudflareBindings): boolean {
  return !!env.POSTHOG_HOST && !!env.POSTHOG_KEY && (env.ENVIRONMENT === 'production' || env.POSTHOG_LOCAL === '1');
}

/** An emitter for this environment. Each event is one capture call, handed to `waitUntil` (or left to run when
    there is none); its failure is swallowed. Off: a no-op. */
export function emitter(env: CloudflareBindings, waitUntil?: (promise: Promise<unknown>) => void, fetcher: typeof fetch = fetch): Emit {
  if (!telemetryOn(env)) return () => {};
  const url = `${env.POSTHOG_HOST!.replace(/\/$/, '')}/i/v0/e/`;
  return (event, distinctId, properties = {}) => {
    const sending = (async () => {
      try {
        await fetcher(url, {
          method: 'POST', headers: { 'content-type': 'application/json' }, signal: AbortSignal.timeout(3000),
          body: JSON.stringify({ api_key: env.POSTHOG_KEY, event, distinct_id: distinctId, timestamp: new Date().toISOString(), properties: { ...properties, $lib: 'powermove-cloud', $process_person_profile: false } }),
        });
      } catch { /* best-effort */ }
    })();
    try { waitUntil?.(sending); } catch { /* no execution context: it runs on its own */ }
  };
}
