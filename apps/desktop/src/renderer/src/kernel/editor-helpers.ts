/** Shared editable-model helpers available to built-in and forked extensions.
 *
 * With the native evaluator on (engine-doc), the evaluation helpers are thin
 * shims over pm-wasm with unchanged signatures (docs/rust-engine.md, section 7,
 * "Editor helpers"): they route when the registry they are handed is the
 * editor's own (it carries `EngineDoc`) and answer with the legacy code
 * otherwise. Factories, drawing, and edits (`makeVectorPath`, `tracePath`,
 * `temporalKeys`, `enableTimeRemap`) stay in TypeScript until pm-edit. */
import { isProperty, canAnimateContent, contentLabel, resolveContent as legacyResolveContent, evaluatedValue as legacyEvaluatedValue } from '../legacy/core/content-properties';
import { makeVectorPath, makeVertex, structuredProperties, pathValues as legacyPathValues, groupMatrix as legacyGroupMatrix, tracePath, pathTargets } from '../legacy/core/vector-paths';
import { validMatteSource as legacyValidMatteSource, MATTE_MODES } from '../legacy/core/matte';
import { expressionDiagnostic as legacyExpressionDiagnostic, EXPRESSION_NAMES } from '../legacy/core/expression';
import { CHANNELS_3D, is3DLayer as legacyIs3DLayer, planeMatrix as legacyPlaneMatrix, planeContains as legacyPlaneContains, projectPoint as legacyProjectPoint, inversePlane as legacyInversePlane } from '../legacy/core/space-3d';
import { countTextUnits as legacyCountTextUnits } from '../legacy/core/text-animation';
import type { NativeApi } from '../engine-doc/native';

export { isProperty, canAnimateContent, contentLabel };
export { makeVectorPath, makeVertex, structuredProperties, tracePath, pathTargets };
export { temporalKeys } from '../legacy/core/temporal-bridge';
export { enableTimeRemap } from '../legacy/core/retiming';
export { MATTE_MODES, EXPRESSION_NAMES, CHANNELS_3D };
export { axisContentKey, axisPath, isAxisTag, inspectFont } from '../typography/font-catalog';
export type { FontAxis, FontInspection, FontStyle } from '../typography/font-catalog';
export { propertyShortcuts } from './property-shortcuts';
/* Captions: pure cue lookups and presets shared by the timeline and inspector. */
export { CAPTION_PRESETS, MIN_CUE_DURATION, cueIndexAt, cueRange, moveLimits, normalizeCaptionStyle, presetStyle } from '../captions/model';
export type { CaptionCue, CaptionStyle, CaptionWord, CaptionsContent } from '../captions/model';
export { mountOverlayOnBody, anchorPicker } from '../controls/overlay';
export { TEXT_ANIMATOR_PROPERTIES, animatorMode, staggerLength, staggerWindow } from '../legacy/core/text-animation';
export type { TextAnimatorMode, TextAnimatorOrder, TextAnimatorProperty, TextAnimatorUnit } from '../legacy/core/text-animation';

/** The native evaluator behind a registry, when it is the editor's own and the flag is on. */
const nativeOf = (PM: any): NativeApi | null => (PM?.EngineDoc?.enabled && PM.EngineDoc.api ? PM.EngineDoc.api : null);
/** The editor's registry (for helpers that take no registry). */
const editorNative = (): NativeApi | null => nativeOf((globalThis as any).PM);

export const resolveContent: typeof legacyResolveContent = (PM, layer, time) => {
  const n = nativeOf(PM);
  return n ? n.resolveContent(layer, time) : legacyResolveContent(PM, layer, time);
};

export const evaluatedValue: typeof legacyEvaluatedValue = (PM, layer, value, time, path) => {
  const n = nativeOf(PM);
  return n ? n.evaluatedValue(layer, value, time, path) : legacyEvaluatedValue(PM, layer, value, time, path);
};

export const pathValues: typeof legacyPathValues = (PM, layer, path, time, ...prefix) => {
  const n = nativeOf(PM);
  return n ? n.pathValues(layer, path, time, ...prefix) : legacyPathValues(PM, layer, path, time, ...prefix);
};

export const groupMatrix: typeof legacyGroupMatrix = (PM, layer, path, time, ...seen) => {
  const n = nativeOf(PM);
  return n && !seen.length ? n.groupMatrix(layer, path, time) : legacyGroupMatrix(PM, layer, path, time, ...seen);
};

export const validMatteSource: typeof legacyValidMatteSource = (layers, layer, id) => {
  const n = editorNative();
  return n ? n.validMatteSource(layers, layer, id) : legacyValidMatteSource(layers, layer, id);
};

export const expressionDiagnostic: typeof legacyExpressionDiagnostic = (source) => {
  const n = editorNative();
  return n ? n.expressionDiagnostic(source as string) : legacyExpressionDiagnostic(source);
};

export const is3DLayer: typeof legacyIs3DLayer = (PM, layer) => {
  const n = nativeOf(PM);
  return n ? n.is3DLayer(layer) : legacyIs3DLayer(PM, layer);
};

export const planeMatrix: typeof legacyPlaneMatrix = (PM, L, T, positioning) => {
  const n = nativeOf(PM);
  return (n ? n.planeMatrix(L, T, positioning) : legacyPlaneMatrix(PM, L, T, positioning)) as ReturnType<typeof legacyPlaneMatrix>;
};

export const planeContains: typeof legacyPlaneContains = (PM, L, T, x, y, b) => {
  const n = nativeOf(PM);
  return n ? n.planeContains(L, T, x, y, b) : legacyPlaneContains(PM, L, T, x, y, b);
};

export const projectPoint: typeof legacyProjectPoint = (m, p) => {
  const n = editorNative();
  return n ? n.projectPoint(m, p) : legacyProjectPoint(m, p);
};

export const inversePlane: typeof legacyInversePlane = (m) => {
  const n = editorNative();
  return (n ? n.inversePlane(m) : legacyInversePlane(m)) as ReturnType<typeof legacyInversePlane>;
};

export const countTextUnits: typeof legacyCountTextUnits = (lines, unit) => {
  const n = editorNative();
  return n ? n.countTextUnits(lines, unit) : legacyCountTextUnits(lines, unit);
};
