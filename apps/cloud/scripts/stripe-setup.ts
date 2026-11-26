/* Creates the Powermove products and monthly prices in Stripe, once: a price is found by its lookup key and
   created only when missing, so re-running changes nothing. Prints the price ids for STRIPE_PRICE_AEROLITE and
   STRIPE_PRICE_AERO (the Worker also finds them by lookup key without those).

   STRIPE_SECRET_KEY=sk_test_… bun run stripe:setup            (a test-mode account)
   STRIPE_SECRET_KEY=… STRIPE_API_BASE=http://localhost:12111 bun run stripe:setup   (the local fake)
   A live key is refused unless --live is passed. */
import { stripeClient } from '../src/billing/stripe';
import { PLANS } from '../src/billing/plans';

const key = process.env.STRIPE_SECRET_KEY;
if (!key) throw new Error('Set STRIPE_SECRET_KEY.');
if (key.startsWith('sk_live_') && !process.argv.includes('--live')) throw new Error('A live key: pass --live to set up the live account.');
const stripe = stripeClient({ STRIPE_SECRET_KEY: key, STRIPE_API_BASE: process.env.STRIPE_API_BASE } as CloudflareBindings);

for (const plan of PLANS) {
  const existing = (await stripe.prices.list({ lookup_keys: [plan.lookupKey], active: true, limit: 1 })).data[0];
  if (existing) {
    console.log(`${plan.envPrice}=${existing.id}   (exists: ${plan.lookupKey})`);
    continue;
  }
  const product = await stripe.products.create({ name: `Powermove ${plan.name}`, metadata: { plan: plan.id } }, { idempotencyKey: `powermove-product-${plan.lookupKey}` });
  const price = await stripe.prices.create({
    product: product.id, currency: 'usd', unit_amount: plan.priceCents, recurring: { interval: 'month' },
    lookup_key: plan.lookupKey, tax_behavior: 'exclusive', metadata: { plan: plan.id },
  }, { idempotencyKey: `powermove-price-${plan.lookupKey}` });
  console.log(`${plan.envPrice}=${price.id}   (created: ${plan.lookupKey})`);
}
