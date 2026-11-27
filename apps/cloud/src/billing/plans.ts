/* The plans the website lists (apps/www/src/lib/Pricing.svelte). Free is the absence of a plan: no credits.
   `forSale`: offered and bought now. Aero isn't yet (owner, 2026-11-26); its subscriptions, if any, still resolve. */
export type PlanId = 'aerolite' | 'aero';
/** `lookupKey` is only what stripe-setup names the price it creates; the Worker never resolves a plan by it. */
export interface Plan { id: PlanId; name: string; priceCents: number; credits: number; lookupKey: string; envPrice: 'STRIPE_PRICE_AEROLITE' | 'STRIPE_PRICE_AERO'; forSale: boolean }
export const PLANS: readonly Plan[] = [
  { id: 'aerolite', name: 'Aerolite', priceCents: 999, credits: 1000, lookupKey: 'powermove_aerolite_monthly', envPrice: 'STRIPE_PRICE_AEROLITE', forSale: true },
  { id: 'aero', name: 'Aero', priceCents: 1999, credits: 2500, lookupKey: 'powermove_aero_monthly', envPrice: 'STRIPE_PRICE_AERO', forSale: false },
];
export const isPlanId = (value: unknown): value is PlanId => PLANS.some(plan => plan.id === value);
export const isForSale = (value: unknown): value is PlanId => PLANS.some(plan => plan.id === value && plan.forSale);
export const planById = (id: PlanId): Plan => PLANS.find(plan => plan.id === id)!;

/** The plan a Stripe price sells: only by the exact price id configured for it (STRIPE_PRICE_*). Lookup keys and
    metadata are anyone's to set on a price in the account, so they never decide a plan. */
export function planForPrice(env: CloudflareBindings, price: { id: string } | string): Plan | null {
  const id = typeof price === 'string' ? price : price.id;
  return PLANS.find(plan => env[plan.envPrice] !== undefined && env[plan.envPrice] !== '' && env[plan.envPrice] === id) ?? null;
}
