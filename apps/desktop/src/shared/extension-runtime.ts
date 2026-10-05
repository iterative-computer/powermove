/** Named runtime exports for the public powermove module. Keep in sync with editor-helpers.ts. */
export const EDITOR_HELPER_EXPORTS = ['isProperty', 'canAnimateContent', 'resolveContent', 'contentLabel', 'evaluatedValue', 'makeVectorPath', 'makeVertex', 'structuredProperties', 'pathValues', 'groupMatrix', 'tracePath', 'pathTargets', 'temporalKeys', 'enableTimeRemap', 'validMatteSource', 'MATTE_MODES', 'expressionDiagnostic', 'EXPRESSION_NAMES', 'axisContentKey', 'axisPath', 'isAxisTag', 'inspectFont', 'CHANNELS_3D', 'is3DLayer', 'planeMatrix', 'planeContains', 'projectPoint', 'inversePlane', 'propertyShortcuts', 'mountOverlayOnBody', 'anchorPicker', 'TEXT_ANIMATOR_PROPERTIES', 'animatorMode', 'countTextUnits', 'staggerLength', 'staggerWindow'] as const;

/**
 * Svelte entry points an extension bundle may import: every public client-side
 * subpath of svelte 5 plus the internal client the compiler emits. Server,
 * compiler, legacy-only and type-only (`action`, `elements`) entries are left
 * out. The compiler, the editor's runtime table and the sandbox's all key on
 * this list, so a specifier resolves everywhere or nowhere.
 */
export const SVELTE_RUNTIME_MODULES = ['svelte', 'svelte/animate', 'svelte/attachments', 'svelte/easing', 'svelte/events', 'svelte/internal/client', 'svelte/motion', 'svelte/reactivity', 'svelte/reactivity/window', 'svelte/store', 'svelte/transition'] as const;
export type SvelteRuntimeModule = typeof SVELTE_RUNTIME_MODULES[number];
