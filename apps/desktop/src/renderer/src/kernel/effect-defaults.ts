import type { EffectParamDefinition } from './api';

export function effectParameterDefault(param: EffectParamDefinition, size: { width: number; height: number }): number | string | boolean {
  if ((!param.type || param.type === 'number') && param.defaultFrom) {
    const dimension = param.defaultFrom.dimension === 'width' ? size.width : size.height;
    const value = dimension * param.defaultFrom.factor;
    if (Number.isFinite(dimension) && dimension > 0 && Number.isFinite(value)) return value;
  }
  return param.def;
}

export function effectDefaultDimensions(layer: any, project: any, bounds?: { w?: number; h?: number } | null): { width: number; height: number } {
  const positive = (...values: unknown[]) => {
    for (const value of values) {
      const number = Number(value && typeof value === 'object' ? (value as any).v : value);
      if (Number.isFinite(number) && number > 0) return number;
    }
    return 1;
  };
  return { width: positive(bounds?.w, layer?.d?.w, project?.w), height: positive(bounds?.h, layer?.d?.h, project?.h) };
}

export function validEffectParameterDefault(param: EffectParamDefinition): boolean {
  const context = param.defaultFrom;
  return context == null || ((!param.type || param.type === 'number')
    && ['width','height'].includes(context.dimension) && Number.isFinite(context.factor) && Math.abs(context.factor) <= 1e6);
}
