/** Shared editable-model helpers available to built-in and forked extensions. */
export { isProperty, canAnimateContent, resolveContent, contentLabel, evaluatedValue } from '../legacy/core/content-properties';
export { makeVectorPath, makeVertex, structuredProperties, pathValues, groupMatrix, tracePath, pathTargets } from '../legacy/core/vector-paths';
export { temporalKeys } from '../legacy/core/temporal-bridge';
export { enableTimeRemap } from '../legacy/core/retiming';
export { validMatteSource, MATTE_MODES } from '../legacy/core/matte';
export { expressionDiagnostic, EXPRESSION_NAMES } from '../legacy/core/expression';
export { axisContentKey, axisPath, isAxisTag, inspectFont } from '../typography/font-catalog';
export { CHANNELS_3D, is3DLayer, planeMatrix, planeContains, projectPoint, inversePlane } from '../legacy/core/space-3d';
export type { FontAxis, FontInspection, FontStyle } from '../typography/font-catalog';
export { propertyShortcuts } from './property-shortcuts';
/* Captions: pure cue lookups and presets shared by the timeline and inspector. */
export { CAPTION_PRESETS, MIN_CUE_DURATION, cueIndexAt, cueRange, moveLimits, normalizeCaptionStyle, presetStyle } from '../captions/model';
export type { CaptionCue, CaptionStyle, CaptionWord, CaptionsContent } from '../captions/model';
export { mountOverlayOnBody, anchorPicker } from '../controls/overlay';
export { TEXT_ANIMATOR_PROPERTIES, animatorMode, countTextUnits, staggerLength, staggerWindow } from '../legacy/core/text-animation';
export type { TextAnimatorMode, TextAnimatorOrder, TextAnimatorProperty, TextAnimatorUnit } from '../legacy/core/text-animation';
