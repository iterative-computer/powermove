/*
 * Aerolite: the paid plan's calls to Powermove Cloud, and the hosted model
 * connection the agent runner uses. The bearer token stays in main; the
 * renderer only sees plan facts.
 */
import { ApiError } from '@powermove/registry/wire';
import { CreatedShareDto, PlanDto, SharesDto, type CreateShareRequest, type PlanTier } from '../../shared/aerolite';
import type { CloudSession } from './session';

const json = (body: unknown): RequestInit => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });

async function parsed<T>(response: Response, schema: { safeParse(v: unknown): { success: true; data: T } | { success: false } }): Promise<T> {
  const result = schema.safeParse(await response.json().catch(() => null));
  if (!result.success) throw new ApiError({ error: 'internal', detail: 'This version of Powermove can’t read the response from Powermove Cloud. Update Powermove, then try again.' });
  return result.data;
}
const signedIn = (session: CloudSession): void => {
  if (!session.currentToken()) throw new ApiError({ error: 'unauthorized', detail: 'Sign in to Powermove first.' });
};

export function createAerolite(session: CloudSession, openExternal: (url: string) => Promise<void>) {
  const call = (path: string, init?: RequestInit) => {
    const client = session.client();
    return client.raw((_api, fetch) => fetch(`${client.origin}${path}`, init));
  };
  /** Only Stripe's own hosts may be opened from a cloud response. */
  const openStripe = async (url: unknown): Promise<void> => {
    const parsedUrl = typeof url === 'string' ? new URL(url) : null;
    if (!parsedUrl || parsedUrl.protocol !== 'https:' || !/(^|\.)stripe\.com$/.test(parsedUrl.hostname)) throw new ApiError({ error: 'internal', detail: 'Billing returned an unexpected address.' });
    await openExternal(parsedUrl.toString());
  };
  return {
    async plan(): Promise<PlanDto | null> {
      if (!session.currentToken()) return null;
      return parsed(await call('/v1/billing/plan'), PlanDto);
    },
    async checkout(tier: PlanTier): Promise<void> {
      signedIn(session);
      const body = await (await call('/v1/billing/checkout', json({ tier }))).json() as { url?: unknown };
      await openStripe(body.url);
    },
    async portal(): Promise<void> {
      signedIn(session);
      const body = await (await call('/v1/billing/portal', { method: 'POST' })).json() as { url?: unknown };
      await openStripe(body.url);
    },
    async shares() { signedIn(session); return (await parsed(await call('/v1/shares'), SharesDto)).items; },
    async createShare(request: CreateShareRequest) { signedIn(session); return parsed(await call('/v1/shares', json(request)), CreatedShareDto); },
    async revokeShare(id: string): Promise<void> { signedIn(session); await call(`/v1/shares/${encodeURIComponent(id)}`, { method: 'DELETE' }); },
    /** The OpenAI-compatible endpoint and key the agent runner uses for Aerolite models. */
    async hostedConnection(): Promise<{ baseUrl: string; key: string; models: string[]; vision: boolean }> {
      const token = session.currentToken();
      if (!token) throw new Error('Sign in to Powermove to use Aerolite models.');
      const plan = await this.plan().catch((error: unknown) => {
        throw new Error(error instanceof ApiError ? error.body.detail ?? 'Powermove Cloud is unavailable right now.' : 'Powermove Cloud is unavailable right now.');
      });
      if (!plan?.plan.models.length) throw new Error('Built-in models are part of Aerolite. Upgrade in Settings › Plan & Usage.');
      return { baseUrl: `${session.client().origin}/v1/ai`, key: token, models: plan.plan.models.map(m => m.id), vision: true };
    }
  };
}
export type Aerolite = ReturnType<typeof createAerolite>;
