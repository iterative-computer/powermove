interface CloudflareBindings {
  DATABASE_URL: string;
  LOCAL_POSTGRES?: string;
  BETTER_AUTH_SECRET: string;
  GOOGLE_CLIENT_ID?: string;
  GOOGLE_CLIENT_SECRET?: string;
  GOOGLE_DISCOVERY_URL?: string;
  EMAIL_FROM: string;
  DEV_LOG_OTP?: string;
  OTP_SENDER?: (email: string, otp: string) => Promise<void>;
  TURNSTILE_ENABLED?: string;
  TURNSTILE_CLEARANCE_DAYS?: string;
  TURNSTILE_SITE_KEY?: string;
  TURNSTILE_SECRET_KEY?: string;
  /* Aerolite. Secrets: set with `wrangler secret put`, never in wrangler.jsonc. */
  AEROLITE_ANTHROPIC_API_KEY?: string;
  AEROLITE_OPENAI_API_KEY?: string;
  AEROLITE_ANTHROPIC_BASE_URL?: string;
  AEROLITE_OPENAI_BASE_URL?: string;
  /** Test hook: replaces fetch for upstream model calls. */
  AEROLITE_UPSTREAM_FETCH?: typeof fetch;
  STRIPE_SECRET_KEY?: string;
  STRIPE_WEBHOOK_SECRET?: string;
  STRIPE_PRICE_AEROLITE?: string;
  STRIPE_PRICE_AERO?: string;
  /** Test hook: replaces fetch for Stripe API calls. */
  STRIPE_FETCH?: typeof fetch;
}
