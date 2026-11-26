/* The plans the website sells (apps/www/src/lib/Pricing.svelte). Free is the absence of a plan: no credits. */
export type PlanId = 'aerolite' | 'aero';
export interface Plan { id: PlanId; name: string; priceCents: number; credits: number; lookupKey: string; envPrice: 'STRIPE_PRICE_AEROLITE' | 'STRIPE_PRICE_AERO' }
export const PLANS: readonly Plan[] = [
  { id: 'aerolite', name: 'Aerolite', priceCents: 999, credits: 1000, lookupKey: 'powermove_aerolite_monthly', envPrice: 'STRIPE_PRICE_AEROLITE' },
  { id: 'aero', name: 'Aero', priceCents: 1999, credits: 2500, lookupKey: 'powermove_aero_monthly', envPrice: 'STRIPE_PRICE_AERO' },
];
export const isPlanId = (value: unknown): value is PlanId => PLANS.some(plan => plan.id === value);
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
