/*
 * The project's color settings (docs/rust-engine.md section 9, "Project color
 * settings"; Settings › Project › Color), stored at the project root as
 * `color: { mode, linearBlending, blendDomain }` (the engine's
 * ProjectColor, shared/engine-protocol/ProjectColor.ts).
 *
 * An absent field is the default (Standard, linear blending off,
 * HDR-safe blend formulas): migrated projects open that way and are
 * never rewritten until a setting changes. Writing the default back removes
 * the field, so a project that returns to the default looks migrated again
 * and opens in either engine (the `requiresEngine` stamp follows it).
 */
import type { ProjectColor } from './engine-protocol/ProjectColor';

/** What Settings › Project › Color edits. */
export interface ProjectColorSettings {
  linearBlending: boolean;
}

export type StoredProjectColor = Omit<ProjectColor, 'precision'>;

export const DEFAULT_PROJECT_COLOR: ProjectColor = {
  mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe',
};

const isRecord = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

/** Upgrade a saved project once, before it enters the editor or document mirror.
 * An explicit Color depth always wins, including 8-bit. The old precision is
 * then removed so future writes have a single source of truth. */
export function migrateProjectColorDepth<T extends Record<string, any>>(project: T): T {
  const color = isRecord(project.color) ? project.color : undefined;
  const legacy = color?.precision;
  const colorDepth = project.colorDepth === undefined
    ? legacy === 'float32' ? 32 : 16
    : project.colorDepth === 8 || project.colorDepth === 32 ? project.colorDepth : 16;
  const result: Record<string, unknown> = { ...project, colorDepth };
  if (color && 'precision' in color) {
    const remaining = { ...color };
    delete remaining.precision;
    if (Object.keys(remaining).length) result.color = remaining;
    else delete result.color;
  }
  return result as T;
}

/** The project's color settings with defaults filled in. */
export function readProjectColor(project: unknown): ProjectColor {
  const color = isRecord(project) ? project['color'] : undefined;
  // Generated Precision.ts gains `unorm8` with the protocol update.
  const precision = (isRecord(project) && project['colorDepth'] === 8 ? 'unorm8'
    : isRecord(project) && project['colorDepth'] === 32 ? 'float32' : 'float16') as ProjectColor['precision'];
  if (!isRecord(color)) return { ...DEFAULT_PROJECT_COLOR, precision };
  return {
    mode: 'standard',
    linearBlending: color['linearBlending'] === true,
    precision,
    blendDomain: color['blendDomain'] === 'displayEncoded' ? 'displayEncoded' : 'hdrSafe',
  };
}

/** Whether the stored field differs from the default (a native-only feature). */
export function hasProjectColor(project: unknown): boolean {
  if (!isRecord(project) || project['color'] == null) return false;
  const color = readProjectColor(project);
  return color.linearBlending !== DEFAULT_PROJECT_COLOR.linearBlending
    || color.blendDomain !== DEFAULT_PROJECT_COLOR.blendDomain;
}

/**
 * The stored field after `patch`, or `undefined` when the result is the
 * default (the field is then removed). `null` when nothing changes.
 */
export function nextProjectColor(project: unknown, patch: Partial<ProjectColorSettings>): StoredProjectColor | undefined | null {
  const current = readProjectColor(project);
  const next: ProjectColor = {
    ...current,
    ...(patch.linearBlending !== undefined ? { linearBlending: patch.linearBlending === true } : {}),
  };
  if (next.linearBlending === current.linearBlending) return null;
  const isDefault = next.linearBlending === DEFAULT_PROJECT_COLOR.linearBlending
    && next.blendDomain === DEFAULT_PROJECT_COLOR.blendDomain;
  if (isDefault) return undefined;
  return { mode: next.mode, linearBlending: next.linearBlending, blendDomain: next.blendDomain };
}
