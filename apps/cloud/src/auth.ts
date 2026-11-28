import { betterAuth } from 'better-auth';
import { drizzleAdapter } from 'better-auth/adapters/drizzle';
import { bearer, emailOTP, genericOAuth, oneTimeToken, username } from 'better-auth/plugins';
import { type Data, tables } from './db/client';
import { HANDLE_RE, isReserved } from './handles';
import { otpEmail } from './email';
export function createAuth(data: Data, env: CloudflareBindings, trackOtpDelivery?: (delivery: Promise<boolean>) => void) {
  return betterAuth({
    database: drizzleAdapter(data.authDb(), { provider: 'pg', schema: tables }),
    baseURL: env.APP_ORIGIN,
    basePath: '/v1/auth',
    secret: env.BETTER_AUTH_SECRET,
    emailAndPassword: { enabled: false },
    plugins: [
      ...(env.GOOGLE_CLIENT_ID && env.GOOGLE_CLIENT_SECRET ? [genericOAuth({ config: [{
        providerId: 'google',
        discoveryUrl: env.GOOGLE_DISCOVERY_URL ?? 'https://accounts.google.com/.well-known/openid-configuration',
        clientId: env.GOOGLE_CLIENT_ID,
        clientSecret: env.GOOGLE_CLIENT_SECRET,
        scopes: ['openid', 'email', 'profile'],
        pkce: true,
        redirectURI: `${env.APP_ORIGIN}/v1/auth/oauth2/callback/google`,
      }] })] : []),
      username({
        minUsernameLength: 2,
        maxUsernameLength: 39,
        usernameValidator: (h) => HANDLE_RE.test(h) && !isReserved(h),
        immutableUsername: true,
      }),
      emailOTP({
        expiresIn: 300,
        otpLength: 6,
        allowedAttempts: 3,
        storeOTP: 'hashed',
        sendVerificationOTP: async ({ email, otp }) => {
          const delivery = (async () => {
            if (env.DEV_LOG_OTP === '1') {
              console.log(`OTP for ${email}: ${otp}`);
              return true;
            }
            if (env.OTP_SENDER) {
              await env.OTP_SENDER(email, otp);
              return true;
            }
            if (!env.EMAIL) {
              throw new Error('email delivery unavailable');
            }
            try {
              await env.EMAIL.send({ to: email, from: env.EMAIL_FROM, ...otpEmail(otp) });
            } catch (error) {
              console.error('email send failed', (error as { code?: string }).code ?? 'UNKNOWN');
              return false;
            }
            return true;
          })().catch(() => false);
          trackOtpDelivery?.(delivery);
          await delivery;
        },
      }),
      bearer(),
      oneTimeToken({ expiresIn: 3, storeToken: 'hashed' }),
    ],
    session: { expiresIn: 60 * 60 * 24 * 90 },
    // Google's state stays single-use and bound to its verification row; the
    // browser cookie check is off because some browsers load the start link
    // twice, and the cookie from the second load breaks the first flow. The
    // desktop and web hand-offs are bound by their own PKCE verifier instead.
    account: { accountLinking: { enabled: true, trustedProviders: ['google'] }, skipStateCookieCheck: true },
  });
}
export type AuthInstance = ReturnType<typeof createAuth>;
