interface CloudflareBindings {
  /** PostHog (the desktop app's project): host and public project key. See telemetry.ts. */
  POSTHOG_HOST?: string;
  POSTHOG_KEY?: string;
  /** '1' lets a non-production Worker send PostHog events. */
  POSTHOG_LOCAL?: string;
  /** 'production' on the deployed Worker (wrangler.jsonc); 'development' locally (.dev.vars). See production.ts. */
  ENVIRONMENT?: string;
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
  /* Billing (v2). Secrets: STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET. Prices by id, or found by lookup key. */
  STRIPE_SECRET_KEY?: string;
  STRIPE_WEBHOOK_SECRET?: string;
  STRIPE_PRICE_AEROLITE?: string;
  STRIPE_PRICE_AERO?: string;
  /** Development only: the local fake Stripe (loopback hosts only). */
  STRIPE_API_BASE?: string;
  /* Aero inference. Secrets: OPENAI_API_KEY, ANTHROPIC_API_KEY. Base URLs point at the local simulator. */
  OPENAI_API_KEY?: string;
  ANTHROPIC_API_KEY?: string;
  AERO_OPENAI_BASE_URL?: string;
  AERO_ANTHROPIC_BASE_URL?: string;
  /** One Durable Object per Aero call (src/aero/call.ts): it owns the provider stream and settles the call. */
  AERO_CALLS?: DurableObjectNamespace;
  /** Per-IP edge limit for Aero and billing (Workers Rate Limiting). */
  RL_AERO?: RateLimit;
  /** The most Aero may spend across all accounts in a UTC day, in USD (default 50). */
  AERO_GLOBAL_DAILY_USD?: string;
  /** What Aero costs, a secret (aero/pricing.ts has the shape). */
  AERO_PRICING?: string;
  /** Sign-in codes sent across all addresses in a UTC day (default 150, below the mail quota). */
  EMAIL_DAILY_LIMIT?: string;
}
