import { betterAuth } from 'better-auth';
import { drizzleAdapter } from 'better-auth/adapters/drizzle';
import { bearer, emailOTP, genericOAuth, oneTimeToken, username } from 'better-auth/plugins';
import { type Data, tables } from './db/client';
import { HANDLE_RE, isReserved } from './handles';
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
        sendVerificationOTP: async ({ email, otp }) => {
          const delivery = (async () => {
            if (env.OTP_SENDER) {
              await env.OTP_SENDER(email, otp);
              return true;
            }
            if (!env.EMAIL) {
              if (env.DEV_LOG_OTP === '1') {
                console.log(`OTP for ${email}: ${otp}`);
                return true;
              }
              throw new Error('email delivery unavailable');
            }
            const text = `Your Powermove sign-in code is ${otp}. It expires in 5 minutes. If you didn't ask for this, ignore it.`;
            try {
              await env.EMAIL.send({
                to: email, from: env.EMAIL_FROM, subject: 'Your Powermove sign-in code', text,
                html: `<p>Your Powermove sign-in code is <strong>${otp}</strong>.</p><p>It expires in 5 minutes. If you didn't ask for this, ignore it.</p>`,
              });
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
    account: { accountLinking: { enabled: true, trustedProviders: ['google'] } },
  });
}
export type AuthInstance = ReturnType<typeof createAuth>;
