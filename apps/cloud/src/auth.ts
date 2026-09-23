import { betterAuth } from 'better-auth';
import { drizzleAdapter } from 'better-auth/adapters/drizzle';
import { username, emailOTP, bearer, oneTimeToken } from 'better-auth/plugins';
import { tables, type Data } from './db/client';
import { HANDLE_RE, isReserved } from './handles';
export function createAuth(data: Data, env: CloudflareBindings) {
  const socialProviders = {
    ...(env.GOOGLE_CLIENT_ID && env.GOOGLE_CLIENT_SECRET ? { google: { clientId: env.GOOGLE_CLIENT_ID, clientSecret: env.GOOGLE_CLIENT_SECRET } } : {}),
    ...(env.GITHUB_CLIENT_ID && env.GITHUB_CLIENT_SECRET ? { github: { clientId: env.GITHUB_CLIENT_ID, clientSecret: env.GITHUB_CLIENT_SECRET } } : {})
  };
  return betterAuth({
    database: drizzleAdapter(data.authDb(), { provider: 'pg', schema: tables }),
    baseURL: env.APP_ORIGIN, basePath: '/v1/auth', secret: env.BETTER_AUTH_SECRET,
    emailAndPassword: { enabled: false }, socialProviders,
    plugins: [username({ minUsernameLength: 2, maxUsernameLength: 39, usernameValidator: h => HANDLE_RE.test(h) && !isReserved(h), immutableUsername: true }),
      emailOTP({ sendVerificationOTP: async ({ email, otp }) => {
        if (!env.RESEND_API_KEY) { console.log(`PENDING(provision) OTP for ${email}: ${otp}`); return; }
        const res = await fetch('https://api.resend.com/emails', { method: 'POST', headers: { Authorization: `Bearer ${env.RESEND_API_KEY}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ from: `Powermove <auth@${env.HANDLE_MAIL_DOMAIN}>`, to: [email], subject: 'Your Powermove code', text: `Your code is ${otp}` }) });
        if (!res.ok) throw new Error('email delivery failed');
      } }), bearer(), oneTimeToken({ expiresIn: 3, storeToken: 'hashed' })],
    session: { expiresIn: 60 * 60 * 24 * 90 },
    account: { accountLinking: { enabled: true, trustedProviders: ['google', 'github'] } }
  });
}
export type AuthInstance = ReturnType<typeof createAuth>;
