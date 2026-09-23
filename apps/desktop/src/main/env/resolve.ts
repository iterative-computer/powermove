/*
 * Which of an extension's declared values are usable (store plan §2.2b).
 * Store 1.0 resolves from the extension's own file only; global values with
 * consent are Store 1.1.
 */
import type { ExtensionVarDecl } from '../../shared/extensions';
import type { VarsResolutionStatus } from '../../shared/vars-ipc';
import type { EnvEntry } from './store';

export interface VarsResolution {
  /** Declared keys with a usable, non-empty value. Undeclared keys are never delivered. */
  values: Record<string, string>;
  /** Required keys without a usable value (unset, empty or undecryptable). */
  missingRequired: string[];
  /** Declared keys whose stored value this Mac cannot decrypt. */
  undecryptable: string[];
  status: VarsResolutionStatus;
}

export function resolveVars(
  decls: readonly ExtensionVarDecl[] | undefined,
  env: ReadonlyMap<string, EnvEntry>
): VarsResolution {
  const values: Record<string, string> = {};
  const missingRequired: string[] = [];
  const undecryptable: string[] = [];
  for (const decl of decls ?? []) {
    const entry = env.get(decl.key);
    if (entry && entry.value === null) undecryptable.push(decl.key);
    const value = entry?.value;
    if (typeof value === 'string' && value.length > 0) values[decl.key] = value;
    else if (decl.required) missingRequired.push(decl.key);
  }
  return {
    values,
    missingRequired,
    undecryptable,
    // A value that can no longer be opened must be entered again, required or not.
    status: missingRequired.length || undecryptable.length ? 'needs-setup' : 'ok'
  };
}

/** Keys to name in `{ state: 'needs-setup', missing }`. */
export function missingKeys(resolution: Pick<VarsResolution, 'missingRequired' | 'undecryptable'>): string[] {
  return [...new Set([...resolution.missingRequired, ...resolution.undecryptable])];
}
