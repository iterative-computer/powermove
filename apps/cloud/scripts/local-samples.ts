import { resolve } from 'node:path';
import { Pool } from 'pg';
import { publishExtensions, type ExtensionListing, type ExtensionPlan } from '../../desktop/scripts/lib/publish-builtins';

type Fetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;

const root = resolve(import.meta.dir, '../samples');
const origin = 'http://localhost:8787';
export const sampleListings: Record<string, ExtensionListing> = {
  'glass-tint': { name: 'Glass Tint', tagline: 'A soft, grainy colour wash for any layer.', category: 'effects', licence: 'MIT', visibility: 'public' },
  'ease-lab': { name: 'Ease Lab', tagline: 'Preview easing curves in a dockable panel.', category: 'panels', licence: 'MIT', visibility: 'public' },
  'colour-match': { name: 'Colour Match', tagline: 'Try a network command with private setup variables.', category: 'commands', licence: 'MIT', visibility: 'public' },
  'wipe-set': { name: 'Wipe Set', tagline: 'Three simple wipes for incoming layers.', category: 'transitions', licence: 'MIT', visibility: 'public' },
};

export function sampleDirs(update = false): string[] {
  const names = ['glass-tint@1.0.0', ...(update ? ['glass-tint@1.1.0'] : []), 'ease-lab', 'colour-match', 'wipe-set'];
  return names.map((name) => resolve(root, name));
}

export async function publishLocalSamples(options: { token: string; update?: boolean; fetch?: Fetch; origin?: string }): Promise<ExtensionPlan[]> {
  return publishExtensions({
    fetch: options.fetch ?? globalThis.fetch,
    dirs: sampleDirs(options.update),
    handle: 'mara', token: options.token, origin: options.origin ?? origin,
    listing: (manifest) => {
      const listing = sampleListings[manifest.id];
      if (!listing) throw new Error(`No listing for ${manifest.id}`);
      return { ...listing, name: manifest.name, tagline: manifest.description ?? manifest.name };
    },
    notes: (manifest) => manifest.id === 'glass-tint' && manifest.version === '1.1.0'
      ? 'Adds an Edge control for a bright rim around the tinted layer.'
      : `First release of ${manifest.name}.`,
  });
}

async function localMaraToken(): Promise<string> {
  const pool = new Pool({ connectionString: 'postgres://powermove:powermove@localhost:54329/powermove', max: 1 });
  try {
    const result = await pool.query<{ token: string }>(
      `select s.token from "session" s
       join publishers p on p.user_id = s.user_id
       where p.handle = 'mara' and s.expires_at > now()
       order by s.created_at desc limit 1`,
    );
    if (!result.rows[0]) throw new Error('Mara has no local session. Run `bun run local:seed` first.');
    return result.rows[0].token;
  } finally {
    await pool.end();
  }
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.some((arg) => arg !== '--update')) throw new Error('Usage: bun run local:samples [--update]');
  const health = await fetch(`${origin}/health`).catch(() => null);
  if (!health?.ok) throw new Error(`Worker is not reachable at ${origin}. Run \`bun run dev:local\` first.`);
  const token = process.env.MARA_REGISTRY_TOKEN ?? await localMaraToken();
  const plans = await publishLocalSamples({ token, update: args.includes('--update') });
  for (const plan of plans) console.log(`${plan.status} ${plan.coordinate}@${plan.version}`);
}
