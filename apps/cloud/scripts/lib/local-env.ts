/* One local stack's names and ports. The defaults are the usual stack; a lane or a second checkout sets
   PM_CLOUD_STACK (the Docker Compose project and container prefix) and PM_CLOUD_PORT_BASE to run its own copy
   side by side: Postgres on base+0, the Worker on base+1, its inspector on base+2, the fake Stripe on base+3 and
   the AI simulator on base+4. Without a base: 54329, 8787, 9229, 12111, 4010. */
const base = process.env.PM_CLOUD_PORT_BASE ? Number(process.env.PM_CLOUD_PORT_BASE) : null;
export const stack = process.env.PM_CLOUD_STACK ?? 'powermove-cloud';
export const ports = base
  ? { postgres: base, worker: base + 1, inspector: base + 2, stripe: base + 3, ai: base + 4 }
  : { postgres: 54329, worker: 8787, inspector: 9229, stripe: 12111, ai: 4010 };
export const databaseUrl = `postgres://powermove:powermove@localhost:${ports.postgres}/powermove`;
export const workerUrl = `http://localhost:${ports.worker}`;
/** The environment Docker Compose needs for this stack. */
export const composeEnv = { ...process.env, COMPOSE_PROJECT_NAME: stack, PM_CLOUD_STACK: stack, PM_CLOUD_PG_PORT: String(ports.postgres) };
