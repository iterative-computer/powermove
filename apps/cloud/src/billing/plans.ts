/* The plans the website lists (apps/www/src/lib/Pricing.svelte). Free is the absence of a plan: no credits.
   `forSale`: offered and bought now. Aero isn't yet (owner, 2026-11-26); its subscriptions, if any, still resolve. */
export type PlanId = 'aerolite' | 'aero';
export interface Plan { id: PlanId; name: string; priceCents: number; credits: number; lookupKey: string; envPrice: 'STRIPE_PRICE_AEROLITE' | 'STRIPE_PRICE_AERO'; forSale: boolean }
export const PLANS: readonly Plan[] = [
  { id: 'aerolite', name: 'Aerolite', priceCents: 999, credits: 1000, lookupKey: 'powermove_aerolite_monthly', envPrice: 'STRIPE_PRICE_AEROLITE', forSale: true },
  { id: 'aero', name: 'Aero', priceCents: 1999, credits: 2500, lookupKey: 'powermove_aero_monthly', envPrice: 'STRIPE_PRICE_AERO', forSale: false },
];
export const isPlanId = (value: unknown): value is PlanId => PLANS.some(plan => plan.id === value);
export const isForSale = (value: unknown): value is PlanId => PLANS.some(plan => plan.id === value && plan.forSale);
export const planById = (id: PlanId): Plan => PLANS.find(plan => plan.id === id)!;

/** The plan a Stripe price sells: by the configured price id, its lookup key, or its `plan` metadata. */
export function planForPrice(env: CloudflareBindings, price: { id: string; lookup_key?: string | null; metadata?: Record<string, string> | null } | string): Plan | null {
  const id = typeof price === 'string' ? price : price.id;
  for (const plan of PLANS) {
    if (env[plan.envPrice] && env[plan.envPrice] === id) return plan;
    if (typeof price !== 'string' && (price.lookup_key === plan.lookupKey || price.metadata?.plan === plan.id)) return plan;
  }
  return null;
}
