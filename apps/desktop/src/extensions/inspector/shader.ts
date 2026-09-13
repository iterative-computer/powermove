import type { Layer, PowermoveAPI } from 'powermove';
import type { ShaderParameterDefinition } from './context';

const definitionsByLayer = new WeakMap<object, ShaderParameterDefinition[]>();

/** Parse inspector annotations without reaching into the renderer's shader registry. */
export function parseUniforms(code: string): ShaderParameterDefinition[] {
  const output: ShaderParameterDefinition[] = [];
  const pattern = /uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(code))) {
    const [, type = '', name = '', annotation = ''] = match;
    if (/^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(name)) continue;
    const values = annotation.trim().split(/\s+/).filter(Boolean);
    const definition: ShaderParameterDefinition = {
      name,
      type,
      label: name.replace(/^u/, '').replace(/([a-z])([A-Z])/g, '$1 $2'),
      control: 'num',
      def: 0
    };
    if (type === 'vec3' && values[0]?.startsWith('#')) {
      definition.control = 'color';
      definition.def = values[0];
    } else if (type === 'bool') {
      definition.control = 'toggle';
      definition.def = values[0] === 'true';
    } else {
      const fallback = Number(values[0]);
      definition.def = Number.isNaN(fallback) ? 0 : fallback;
      definition.min = values[1] !== undefined ? Number(values[1]) : (Number(definition.def) < 0 ? Number(definition.def) * 2 : 0);
      definition.max = values[2] !== undefined ? Number(values[2]) : Math.max(1, Math.abs(Number(definition.def)) * 2);
    }
    output.push(definition);
  }
  return output;
}

export function syncShaderUniforms(api: PowermoveAPI, layer: Layer): void {
  const content = layer.d as Record<string, any>;
  const definitions = parseUniforms(String(content.code ?? ''));
  definitionsByLayer.set(layer, definitions);
  api.uiState.setShaderMeta(layer, { udefs: definitions });
  const uniforms = content.uniforms ?? (content.uniforms = {});
  for (const definition of definitions) {
    if (!uniforms[definition.name]) uniforms[definition.name] = api.model.P(definition.def as never);
  }
  for (const name of Object.keys(uniforms)) {
    if (!definitions.some((definition) => definition.name === name)) delete uniforms[name];
  }
}

export function shaderDefinitions(layer: Layer): ShaderParameterDefinition[] {
  return definitionsByLayer.get(layer) ?? [];
}
