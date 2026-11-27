/* The whole local billing stack in one command: Postgres (Docker Compose), the fake Stripe, the AI simulator,
   the Stripe products (stripe:setup against the fake) and the Worker (wrangler dev), then seed data:
   Jude on Free, Mara on Aerolite with a few days of usage, so the app's billing UI has something to show.
   Nothing leaves this machine; no real keys are read. Ctrl-C stops the Worker and the fakes; Postgres keeps
   running until `bun run local:down`.

   bun run local:billing
   PM_CLOUD_STACK=w113 PM_CLOUD_PORT_BASE=54410 bun run local:billing   (a second stack beside the usual one) */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Pool } from 'pg';
import { databaseUrl, ports, stack, workerUrl } from './lib/local-env';
import { FAKE_SECRET_KEY, FAKE_WEBHOOK_SECRET, startFakeStripe } from './fake-stripe';
import { startAiSimulator } from './ai-simulator';
import { CREDIT_NANO_USD } from '../src/aero/catalogue';

const cloud = resolve(import.meta.dir, '..');
const children: Array<ReturnType<typeof Bun.spawn>> = [];
const run = async (command: string[], env: Record<string, string | undefined> = {}) => {
  const child = Bun.spawn(command, { cwd: cloud, env: { ...process.env, ...env }, stdout: 'pipe', stderr: 'inherit' });
  const out = await new Response(child.stdout).text();
  if (await child.exited !== 0) throw new Error(`${command.join(' ')} failed:\n${out}`);
  return out;
};

console.log(`Stack ${stack}: Postgres :${ports.postgres}, Worker :${ports.worker}, fake Stripe :${ports.stripe}, AI simulator :${ports.ai}`);
process.stdout.write(await run(['bun', 'scripts/local-up.ts']));

const stripe = startFakeStripe({ port: ports.stripe, webhookUrl: `${workerUrl}/v2/stripe/webhook` });
const ai = startAiSimulator({ port: ports.ai });
const billingVars: Record<string, string> = {
  LOCAL_POSTGRES: '1',
  DATABASE_URL: databaseUrl,
  APP_ORIGIN: workerUrl,
  STRIPE_SECRET_KEY: FAKE_SECRET_KEY,
  STRIPE_WEBHOOK_SECRET: FAKE_WEBHOOK_SECRET,
  STRIPE_API_BASE: stripe.url,
  OPENAI_API_KEY: 'local-simulator',
  ANTHROPIC_API_KEY: 'local-simulator',
  AERO_OPENAI_BASE_URL: `${ai.url}/openai/v1`,
  AERO_ANTHROPIC_BASE_URL: `${ai.url}/anthropic/v1`,
};
process.stdout.write(await run(['bun', 'scripts/stripe-setup.ts'], { STRIPE_SECRET_KEY: FAKE_SECRET_KEY, STRIPE_API_BASE: stripe.url }));

// .dev.vars plus this stack's values, in a file of its own (wrangler reads one env file).
const base = await readFile(resolve(cloud, '.dev.vars'), 'utf8');
const varsFile = resolve(cloud, `.wrangler/local-billing/${stack}.vars`);
await mkdir(resolve(varsFile, '..'), { recursive: true });
const kept = base.split('\n').filter(line => !Object.keys(billingVars).some(key => line.startsWith(`${key}=`)));
await writeFile(varsFile, [...kept, ...Object.entries(billingVars).map(([key, value]) => `${key}=${value}`)].join('\n') + '\n');
const worker = Bun.spawn(['bunx', 'wrangler', 'dev', '--port', String(ports.worker), '--inspector-port', String(ports.inspector), '--test-scheduled', '--env-file', varsFile, '--persist-to', `.wrangler/state-${stack}`], {
  cwd: cloud, stdout: 'inherit', stderr: 'inherit',
});
children.push(worker);
const stop = async () => { for (const child of children) child.kill(); await stripe.stop(); await ai.stop(); process.exit(0); };
process.on('SIGINT', () => { void stop(); });
process.on('SIGTERM', () => { void stop(); });

for (let i = 0; ; i++) {
  if ((await fetch(`${workerUrl}/health`).catch(() => null))?.ok) break;
  if (i > 120 || worker.exitCode !== null) throw new Error('The Worker did not start.');
  await Bun.sleep(500);
}

// Seed: Jude and Mara with sessions (local:seed), then Mara through Checkout and a few days of Aero use.
const seed = await run(['bun', 'scripts/local-seed.ts']);
const judeToken = seed.match(/POWERMOVE_REGISTRY_TOKEN=(\S+)/)?.[1];
const maraToken = seed.match(/MARA_REGISTRY_TOKEN=(\S+)/)?.[1];
if (!judeToken || !maraToken) throw new Error('local:seed printed no tokens');
const api = (token: string, path: string, body?: unknown) => fetch(`${workerUrl}${path}`, {
  method: body === undefined ? 'GET' : 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body),
});
let mara = await (await api(maraToken, '/v2/billing')).json() as any;
if (!mara.plan) {
  const { url } = await (await api(maraToken, '/v2/billing/checkout', { plan: 'aerolite' })).json() as { url: string };
  await stripe.pay(url.split('/').at(-1)!, 4);
  mara = await (await api(maraToken, '/v2/billing')).json() as any;
}
if (mara.credits.used === 0) {
  const pool = new Pool({ connectionString: databaseUrl, max: 1 });
  try {
    const { rows } = await pool.query<{ id: string }>(`select id from "user" where email = 'mara@localhost'`);
    const start = new Date(mara.period.start).getTime();
    for (const [day, credits, model] of [[0, 46, 'anthropic/claude-opus-5-5'], [1, 0, null], [2, 92, 'openai/gpt-6.2-sol'], [3, 31, 'openai/gpt-6-luna']] as const) {
      if (!model) continue;
      await pool.query(
        `insert into credit_ledger (user_id, kind, idempotency_key, credits, period_start, period_end, model, input_tokens, fresh_input_tokens, output_tokens, cost_nano_usd, usage_source, created_at)
         values ($1, 'spend', $2, 0, $3, $4, $5, $6, $6, $7, $8, 'reported', $9) on conflict (idempotency_key) do nothing`,
        [rows[0]!.id, `request:${rows[0]!.id}:seed-day-${day}`, mara.period.start, mara.period.end, model, credits * 400, credits * 20, credits * CREDIT_NANO_USD, new Date(start + day * 86_400_000 + 3_600_000)],
      );
    }
  } finally { await pool.end(); }
  for (const model of ['openai/gpt-6.2-sol', 'anthropic/claude-sonnet-5-5']) {
    const r = await api(maraToken, '/v2/aero/chat/completions', { model, stream: true, messages: [{ role: 'system', content: 'You are Aero.' }, { role: 'user', content: 'Hello from the seed.' }] });
    await r.text();
  }
  mara = await (await api(maraToken, '/v2/billing')).json() as any;
}

console.log(`
Local billing stack ${stack} is up.
  Worker        ${workerUrl}
  fake Stripe   ${stripe.url}   (Checkout and Portal pages; POST /_fake/subscriptions/<id>/renew | fail-renewal | end)
  AI simulator  ${ai.url}
  Jude (Free)       ${judeToken}
  Mara (Aerolite)   ${maraToken}   credits ${mara.credits.remaining} of ${mara.credits.included} left
Ctrl-C stops the Worker and the fakes; \`bun run local:down\` removes Postgres.`);
await new Promise(() => {});
