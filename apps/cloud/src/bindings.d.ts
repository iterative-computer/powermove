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
  ADMIN_TOKEN?: string;
  TURNSTILE_ENABLED?: string;
  TURNSTILE_CLEARANCE_DAYS?: string;
  TURNSTILE_SITE_KEY?: string;
  TURNSTILE_SECRET_KEY?: string;
}
