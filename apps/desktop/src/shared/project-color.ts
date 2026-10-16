/*
 * The project's color settings (docs/rust-engine.md section 9, "Project color
 * settings"; Settings › Project › Color), stored at the project root as
 * `color: { mode, linearBlending, precision, blendDomain }` (the engine's
 * ProjectColor, shared/engine-protocol/ProjectColor.ts).
 *
 * An absent field is the default (Standard, linear blending off, 16-bit
 * float, HDR-safe blend formulas): migrated projects open that way and are
 * never rewritten until a setting changes. Writing the default back removes
 * the field, so a project that returns to the default looks migrated again
 * and opens in either engine (the `requiresEngine` stamp follows it).
 */
import type { ProjectColor } from './engine-protocol/ProjectColor';

export type ColorPrecision = ProjectColor['precision'];

/** What Settings › Project › Color edits. */
export interface ProjectColorSettings {
  linearBlending: boolean;
  precision: ColorPrecision;
}

export const DEFAULT_PROJECT_COLOR: ProjectColor = {
  mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe',
};

export const PRECISION_OPTIONS: Array<{ v: ColorPrecision; label: string }> = [
  { v: 'float16', label: '16-bit float' },
  { v: 'float32', label: '32-bit float' },
];

const isRecord = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

/** The project's color settings with defaults filled in. */
export function readProjectColor(project: unknown): ProjectColor {
  const color = isRecord(project) ? project['color'] : undefined;
  if (!isRecord(color)) return { ...DEFAULT_PROJECT_COLOR };
  return {
    mode: 'standard',
    linearBlending: color['linearBlending'] === true,
    precision: color['precision'] === 'float32' ? 'float32' : 'float16',
    blendDomain: color['blendDomain'] === 'displayEncoded' ? 'displayEncoded' : 'hdrSafe',
  };
}

/** Whether the stored field differs from the default (a native-only feature). */
export function hasProjectColor(project: unknown): boolean {
  if (!isRecord(project) || project['color'] == null) return false;
  const color = readProjectColor(project);
  return color.linearBlending !== DEFAULT_PROJECT_COLOR.linearBlending
    || color.precision !== DEFAULT_PROJECT_COLOR.precision
    || color.blendDomain !== DEFAULT_PROJECT_COLOR.blendDomain;
}

/**
 * The stored field after `patch`, or `undefined` when the result is the
 * default (the field is then removed). `null` when nothing changes.
 */
export function nextProjectColor(project: unknown, patch: Partial<ProjectColorSettings>): ProjectColor | undefined | null {
  const current = readProjectColor(project);
  const next: ProjectColor = {
    ...current,
    ...(patch.linearBlending !== undefined ? { linearBlending: patch.linearBlending === true } : {}),
    ...(patch.precision !== undefined ? { precision: patch.precision === 'float32' ? 'float32' : 'float16' } : {}),
  };
  if (next.linearBlending === current.linearBlending && next.precision === current.precision) return null;
  const isDefault = next.linearBlending === DEFAULT_PROJECT_COLOR.linearBlending
    && next.precision === DEFAULT_PROJECT_COLOR.precision
    && next.blendDomain === DEFAULT_PROJECT_COLOR.blendDomain;
  return isDefault ? undefined : next;
}
