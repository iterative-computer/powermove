/*
 * Extension values, joined up: the provenance file names each folder's env
 * key, the env store holds the values, and resolution decides what an
 * extension gets. Main-only; nothing here is renderer-reachable on its own.
 */
import type { ExtensionVarDecl } from '../../shared/extensions';
import type { VarsStatus } from '../../shared/vars-ipc';
import type { ProvenanceStore } from '../cloud/provenance';
import { resolveVars, type VarsResolution } from './resolve';
import type { EnvMap, EnvStore } from './store';

export interface VarsService {
  resolve(id: string, decls: readonly ExtensionVarDecl[]): Promise<VarsResolution>;
  status(id: string, decls: readonly ExtensionVarDecl[]): Promise<VarsStatus>;
  set(id: string, decl: ExtensionVarDecl, value: string): Promise<void>;
  delete(id: string, key: string): Promise<void>;
  /** One stored value, for the native reveal dialog only. */
  read(id: string, decls: readonly ExtensionVarDecl[], key: string): Promise<string | null>;
  /** True when the extension has a values file on disk. */
  hasValues(id: string): Promise<boolean>;
  /** Delete the values file and the provenance entry. */
  forget(id: string): Promise<void>;
}

const secretKeys = (decls: readonly ExtensionVarDecl[]): Set<string> =>
  new Set(decls.filter((decl) => decl.secret).map((decl) => decl.key));

export function createVarsService(options: { env: EnvStore; provenance: ProvenanceStore }): VarsService {
  const { env, provenance } = options;

  async function read(id: string, decls: readonly ExtensionVarDecl[]): Promise<EnvMap> {
    const envKey = (await provenance.get(id))?.envKey;
    return envKey ? env.readEnv(envKey, secretKeys(decls)) : new Map();
  }

  return {
    async resolve(id, decls) {
      return resolveVars(decls, await read(id, decls));
    },

    async status(id, decls) {
      const values = await read(id, decls);
      const resolution = resolveVars(decls, values);
      return {
        keys: decls.map((decl) => {
          const entry = values.get(decl.key);
          return {
            key: decl.key,
            label: decl.label,
            ...(decl.hint ? { hint: decl.hint } : {}),
            secret: decl.secret === true,
            required: false,
            set: typeof entry?.value === 'string' && entry.value.length > 0,
            undecryptable: entry !== undefined && entry.value === null
          };
        }),
        status: resolution.status
      };
    },

    async set(id, decl, value) {
      if (value.length === 0) {
        const existing = (await provenance.get(id))?.envKey;
        if (existing) await env.deleteVar(existing, decl.key);
        return;
      }
      const envKey = await provenance.ensureEnvKey(id);
      await env.writeVar(envKey, decl.key, value, decl.secret === true);
    },

    async delete(id, key) {
      const envKey = (await provenance.get(id))?.envKey;
      if (envKey) await env.deleteVar(envKey, key);
    },

    async read(id, decls, key) {
      const value = (await read(id, decls)).get(key)?.value;
      return typeof value === 'string' && value.length > 0 ? value : null;
    },

    async hasValues(id) {
      const envKey = (await provenance.get(id))?.envKey;
      return envKey ? env.exists(envKey) : false;
    },

    async forget(id) {
      const envKey = (await provenance.get(id))?.envKey;
      if (envKey) await env.deleteEnv(envKey);
      await provenance.remove(id);
    }
  };
}
