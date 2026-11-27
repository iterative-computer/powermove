/* The test pricing (test/fixtures/pricing.json, also the local stacks' AERO_PRICING) and its helpers. */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { AeroModel, Metered } from '../src/aero/catalogue';
import { type Pricing, cost as priced, parsePricing } from '../src/aero/pricing';

/** The fixture as the secret's text. */
export const PRICING_JSON = JSON.stringify(JSON.parse(readFileSync(resolve(import.meta.dir, 'fixtures/pricing.json'), 'utf8')));
export const PRICING: Pricing = parsePricing(PRICING_JSON)!;
/** One credit in nano-USD, under the fixture. */
export const CREDIT_NANO_USD = PRICING.creditNanoUsd;
/** A cost at the fixture's prices. */
export const cost = (model: AeroModel, usage: Metered) => priced(PRICING, model, usage);
