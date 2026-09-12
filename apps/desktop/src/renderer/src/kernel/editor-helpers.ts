/** Shared editable-model helpers available to built-in and forked extensions. */
export { isProperty, canAnimateContent, resolveContent, contentLabel, evaluatedValue } from '../legacy/core/content-properties';
export { makeVectorPath, makeVertex, structuredProperties, pathValues, groupMatrix, tracePath, pathTargets } from '../legacy/core/vector-paths';
export { temporalKeys } from '../legacy/core/temporal-bridge';
export { enableTimeRemap } from '../legacy/core/retiming';
export { validMatteSource, MATTE_MODES } from '../legacy/core/matte';
export { expressionDiagnostic, EXPRESSION_NAMES } from '../legacy/core/expression';
export { axisContentKey, axisPath, isAxisTag, inspectFont } from '../typography/font-catalog';
export type { FontAxis, FontInspection } from '../typography/font-catalog';
