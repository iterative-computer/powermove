import { Hono } from 'hono';
import type { Env } from '../env';
import { verifiedEvent } from '../billing/stripe';
import { processEvent } from '../billing/webhook';
import { WEBHOOK_BODY_BYTES, readLimited } from '../body';

/** Stripe's webhook: signature-verified, idempotent by event id. A failure answers 500 so Stripe retries. */
export const stripeWebhook = new Hono<Env>()
  .post('/', async (c) => {
    // Capped before it is read whole (Stripe's events are a few kilobytes).
    const payload = await readLimited(c, WEBHOOK_BODY_BYTES);
    const event = await verifiedEvent(c.env, payload, c.req.header('stripe-signature'));
    const processed = await processEvent(c.var.data, c.env, event);
    return c.json({ received: true, duplicate: !processed });
  });
