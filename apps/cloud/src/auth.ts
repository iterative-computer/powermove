import { betterAuth } from 'better-auth';
import { drizzleAdapter } from 'better-auth/adapters/drizzle';
import { bearer, emailOTP, oneTimeToken, username } from 'better-auth/plugins';
import { type Data, tables } from './db/client';
import { HANDLE_RE, isReserved } from './handles';
export function createAuth(data: Data, env: CloudflareBindings, trackOtpDelivery?: (delivery: Promise<void>) => void) {
  const socialProviders = {
    ...(env.GOOGLE_CLIENT_ID && env.GOOGLE_CLIENT_SECRET
      ? { google: { clientId: env.GOOGLE_CLIENT_ID, clientSecret: env.GOOGLE_CLIENT_SECRET } }
      : {}),
    ...(env.GITHUB_CLIENT_ID && env.GITHUB_CLIENT_SECRET
      ? { github: { clientId: env.GITHUB_CLIENT_ID, clientSecret: env.GITHUB_CLIENT_SECRET } }
      : {}),
  };
  return betterAuth({
    database: drizzleAdapter(data.authDb(), { provider: 'pg', schema: tables }),
    baseURL: env.APP_ORIGIN,
    basePath: '/v1/auth',
    secret: env.BETTER_AUTH_SECRET,
    emailAndPassword: { enabled: false },
    socialProviders,
    plugins: [
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
              return;
            }
            if (!env.RESEND_API_KEY) {
              if (env.DEV_LOG_OTP === '1') {
                console.log(`OTP for ${email}: ${otp}`);
                return;
              }
              throw new Error('email delivery unavailable');
            }
            const res = await fetch('https://api.resend.com/emails', {
              method: 'POST',
              headers: { Authorization: `Bearer ${env.RESEND_API_KEY}`, 'Content-Type': 'application/json' },
              body: JSON.stringify({
                from: `Powermove <auth@${env.HANDLE_MAIL_DOMAIN}>`,
                to: [email],
                subject: 'Your Powermove code',
                text: `Your code is ${otp}`,
              }),
            });
            if (!res.ok) throw new Error('email delivery failed');
          })();
          trackOtpDelivery?.(delivery);
          await delivery;
        },
      }),
      bearer(),
      oneTimeToken({ expiresIn: 3, storeToken: 'hashed' }),
    ],
    session: { expiresIn: 60 * 60 * 24 * 90 },
    account: { accountLinking: { enabled: true, trustedProviders: ['google', 'github'] } },
  });
}
export type AuthInstance = ReturnType<typeof createAuth>;
