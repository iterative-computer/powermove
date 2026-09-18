/*
 * Persistent store key registry. The renderer's PM.store keys (formerly
 * localStorage "pm.*") map 1:1 onto files under <userData>/store/. Dynamic
 * keys carry an id segment that is validated here so a file path is never
 * derived from an unchecked renderer string.
 */

export const STATIC_KEYS = [
  'projects',
  // Projects that had an editor window at quit, in window-creation order.
  // `openTabs` is the pre-window key it migrates from; it stays registered so
  // an older profile can still be read once.
  'openWindows',
  'openTabs',
  'restoreWindows',
  'projectTrash',
  'takes',
  'workspaces',
  'workspace',
  'workspaceTrash',
  'theme',
  'themeMode',
  'exportOpts',
  'renderQueue',
  'renderPresets',
  'bootVersion',
  'autosave',
  'projectsSection',
  'projectsSort',
  'projectsView',
  'agentScope',
  'agentProvider',
  'agentModel',
  'agentReasoningEffort',
  'agentAccessMode',
  'agentAutoApplyPanels',
  'extensions',
  'activeTheme'
] as const;

export type StaticKey = (typeof STATIC_KEYS)[number];

export const DYNAMIC_PREFIXES = ['project', 'projectJournal', 'projectState', 'projectMeta', 'projectHistory', 'projectWorkspace', 'agentThreads', 'agentModel', 'ext'] as const;
export type DynamicPrefix = (typeof DYNAMIC_PREFIXES)[number];

const ID = /^[A-Za-z0-9_-]{1,120}$/;

export type ParsedKey = { kind: 'static'; key: StaticKey } | { kind: 'dynamic'; prefix: DynamicPrefix; id: string };

/** Returns null for any key that is not in the registry. */
export function parseStoreKey(key: string): ParsedKey | null {
  if ((STATIC_KEYS as readonly string[]).includes(key)) {
    return { kind: 'static', key: key as StaticKey };
  }
  const dot = key.indexOf('.');
  if (dot <= 0) return null;
  const prefix = key.slice(0, dot);
  const id = key.slice(dot + 1);
  if (!(DYNAMIC_PREFIXES as readonly string[]).includes(prefix) || !ID.test(id)) return null;
  return { kind: 'dynamic', prefix: prefix as DynamicPrefix, id };
}

/** File name (no directory) for a validated key. */
export function storeFileName(parsed: ParsedKey): string {
  return parsed.kind === 'static' ? `${parsed.key}.json` : `${parsed.prefix}.${parsed.id}.json`;
}
