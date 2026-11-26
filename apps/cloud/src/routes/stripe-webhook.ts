import { Hono } from 'hono';
import type { Env } from '../env';
import { verifiedEvent } from '../billing/stripe';
import { processEvent } from '../billing/webhook';

/** Stripe's webhook: signature-verified, idempotent by event id. A failure answers 500 so Stripe retries. */
export const stripeWebhook = new Hono<Env>()
  .post('/', async (c) => {
    const payload = await c.req.text();
    const event = await verifiedEvent(c.env, payload, c.req.header('stripe-signature'));
    const processed = await processEvent(c.var.data, c.env, event);
    return c.json({ received: true, duplicate: !processed });
  });
