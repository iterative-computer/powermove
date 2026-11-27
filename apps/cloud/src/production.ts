/* What a production Worker must and must not have. `ENVIRONMENT=production` (wrangler.jsonc) turns the checks on;
   local stacks set `ENVIRONMENT=development` in .dev.vars. */
import { parsePricing } from './aero/pricing';

/** Development-only settings: each one sends mail to a log, trusts a local identity provider, database, Stripe or
    model provider, or lets a script deliver one-time codes. */
export const DEVELOPMENT_ONLY = ['DEV_LOG_OTP', 'OTP_SENDER', 'GOOGLE_DISCOVERY_URL', 'LOCAL_POSTGRES', 'STRIPE_API_BASE', 'AERO_OPENAI_BASE_URL', 'AERO_ANTHROPIC_BASE_URL'] as const;
/** Settings production can't run without. */
export const REQUIRED = ['BETTER_AUTH_SECRET', 'TURNSTILE_SITE_KEY', 'TURNSTILE_SECRET_KEY', 'STRIPE_SECRET_KEY', 'STRIPE_WEBHOOK_SECRET', 'STRIPE_PRICE_AEROLITE'] as const;
/** Cloudflare's published Turnstile test keys (always pass): never live keys. */
const TEST_TURNSTILE = /^[123]x0{20,}/;

/** Why this environment must not serve, if it is production; nothing otherwise. Names only, never values. */
export function productionProblems(env: CloudflareBindings): string[] {
  if (env.ENVIRONMENT !== 'production') return [];
  const raw = env as unknown as Record<string, unknown>;
  const problems: string[] = [];
  for (const name of DEVELOPMENT_ONLY) if (raw[name] !== undefined && raw[name] !== '') problems.push(`${name} set`);
  for (const name of REQUIRED) if (typeof raw[name] !== 'string' || raw[name] === '') problems.push(`${name} missing`);
  for (const name of ['TURNSTILE_SITE_KEY', 'TURNSTILE_SECRET_KEY'] as const) if (typeof raw[name] === 'string' && TEST_TURNSTILE.test(raw[name] as string)) problems.push(`${name} is a test key`);
  if (env.TURNSTILE_ENABLED !== '1') problems.push('TURNSTILE_ENABLED off');
  if (!parsePricing(env.AERO_PRICING)) problems.push('AERO_PRICING missing or malformed');
  if (typeof env.STRIPE_SECRET_KEY === 'string' && !env.STRIPE_SECRET_KEY.startsWith('sk_live_') && !env.STRIPE_SECRET_KEY.startsWith('rk_live_')) problems.push('STRIPE_SECRET_KEY not live');
  return problems;
}
