interface CloudflareBindings {
  DATABASE_URL: string;
  BETTER_AUTH_SECRET: string;
  GOOGLE_CLIENT_ID?: string;
  GOOGLE_CLIENT_SECRET?: string;
  GOOGLE_DISCOVERY_URL?: string;
  EMAIL_FROM: string;
  DEV_LOG_OTP?: string;
  OTP_SENDER?: (email: string, otp: string) => Promise<void>;
  ADMIN_TOKEN?: string;
}
