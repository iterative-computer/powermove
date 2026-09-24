/* The sandbox check as the Store shows it: running it for a Library item in
   this window, and its report as one line per problem. The check itself is
   kernel/sandbox-check.ts; publishing waits on it in the publish sheet. */
import type { ExtensionRecord } from '../../../shared/extensions';
import { bridge } from '../kernel/bridge';
import { recordFor } from '../kernel/extensions.svelte';
import type { InstalledKernel } from '../kernel/install';
import { runSandboxCheck, type SandboxCheckReport } from '../kernel/sandbox-check';
import type { StorePM } from './data';

export type { SandboxCheckReport };

export type SandboxCheckState =
  | { status: 'running' }
  | { status: 'done'; report: SandboxCheckReport }
  | { status: 'error'; message: string };

export const SANDBOX_OK = 'Runs in the sandbox';
export const SANDBOX_SKIPPED = 'Needs full access: installs only after the user trusts it.';
export const SANDBOX_UNAVAILABLE = 'Powermove couldn’t run the sandbox check in this window. Close this and try again.';

async function valuesFor(record: ExtensionRecord): Promise<Record<string, string>> {
  const declared = new Set((record.manifest?.vars ?? []).map((decl) => decl.key));
  const vars = (bridge() as { vars?: { values?(req: { id: string }): Promise<Record<string, string>> } } | undefined)?.vars;
  if (!declared.size || typeof vars?.values !== 'function') return {};
  try {
    const values = await vars.values({ id: record.id });
    return Object.fromEntries(Object.entries(values ?? {}).filter(([key, value]) => declared.has(key) && typeof value === 'string'));
  } catch {
    return {};
  }
}

/** Run the check for an extension on this Mac, as it is built right now. */
export async function checkInSandbox(PM: StorePM, localId: string): Promise<SandboxCheckReport> {
  const kernel = (PM as { Kernel?: InstalledKernel }).Kernel;
  const record = recordFor(localId) ?? kernel?.loader?.records().find((candidate) => candidate.id === localId);
  if (!kernel?.deps || !record) throw new Error(`No extension "${localId}" in this window`);
  return runSandboxCheck(record, { kernel, deps: kernel.deps, vars: await valuesFor(record) });
}

/* What to use instead of a trusted-only namespace, when there is something. */
const ALTERNATIVE: Record<string, string> = {
  render: 'use api.project instead', anim: 'use api.project instead', model: 'use api.project instead',
  selection: 'use api.project.selection() and api.project.select() instead', groups: 'use api.project instead',
  history: 'use api.project.undo() and api.project.redo() instead', edit: 'use api.project.apply instead',
  uiState: 'use api.project instead', space3d: 'use api.project instead', powermove: 'use api.project instead',
  media: 'use api.assets instead', ui: 'build it inside your panel instead'
};

const FULL_ACCESS = '`permissions: ["full-access"]`';
const NETWORK_DIRECTIVES = new Set(['connect-src', 'img-src', 'media-src', 'font-src']);

function permissionLine(hit: SandboxCheckReport['permissionErrors'][number]): string {
  const name = hit.namespace === 'powermove' ? `${hit.member} from 'powermove'` : `api.${hit.namespace}${hit.member ? `.${hit.member}` : ''}`;
  const alternative = ALTERNATIVE[hit.namespace];
  return alternative
    ? `Calls ${name}, which needs full access. Declare ${FULL_ACCESS} or ${alternative}.`
    : `Calls ${name}, which needs full access. Declare ${FULL_ACCESS}.`;
}

function cspLine(hit: SandboxCheckReport['cspViolations'][number]): string {
  const directive = hit.directive.replace(/-elem$|-attr$/, '');
  if (NETWORK_DIRECTIVES.has(directive) && /^(?:https?|wss?):/i.test(hit.blockedUri)) {
    let where = hit.blockedUri;
    try { where = new URL(hit.blockedUri).host || where; } catch { /* keep the raw value */ }
    return `Reaches ${where} without the network permission. Declare \`permissions: ["network"]\`.`;
  }
  if (hit.blockedUri === 'eval') return 'Uses eval or new Function, which the sandbox blocks. Build the code ahead of time instead.';
  return `The sandbox blocked ${hit.blockedUri || 'a request'} (${hit.directive}).`;
}

/** One sentence per problem, most actionable first. Empty when the report is clean or skipped. */
export function sandboxCheckLines(report: SandboxCheckReport): string[] {
  if (report.skipped || report.ok) return [];
  const lines: string[] = [];
  const permission = report.permissionErrors.length > 0;
  const covered = (message: string): boolean => permission && /requires full access/.test(message);
  for (const hit of report.permissionErrors) lines.push(permissionLine(hit));
  if (report.activation !== 'ok' && !covered(report.activation.error)) lines.push(`Failed to start in the sandbox: ${report.activation.error}`);
  for (const hit of report.cspViolations) lines.push(cspLine(hit));
  for (const hit of report.asyncMisuse) lines.push(`Reads the result of api.${hit.member} right away, but it returns a Promise in the sandbox. Await it and set \`apiVersion: 3\`.`);
  for (const panel of report.panels) if (!panel.mounted && !covered(panel.error ?? '')) lines.push(`Panel '${panel.id}' failed to mount: ${panel.error ?? 'unknown error'}`);
  for (const message of report.runtimeErrors) if (!covered(message)) lines.push(`Threw an error: ${message}`);
  return lines.length ? [...new Set(lines)] : ['It didn’t pass the sandbox check. Run it again from the Library for details.'];
}
