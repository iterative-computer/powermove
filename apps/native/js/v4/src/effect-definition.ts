/** Same authored definition as compat v1–3; frag is a main body by default. */
export type EffectParameter = {
  k: string;
  label: string;
  step?: number;
  unit?: string;
  advanced?: boolean;
  options?: { v: number | boolean; label: string }[];
} & (
  | { type?: "number"; def: number; min: number; max: number }
  | { type: "color"; def: string }
  | { type: "toggle"; def: boolean }
);
export interface EffectDefinition {
  id: string;
  label: string;
  group: string;
  params: EffectParameter[];
  frag: string;
  passes?: number;
  keepOrig?: boolean;
  backdrop?: boolean;
  rawShader?: boolean;
  viewportSafe?: boolean;
  viewportPadding?: string[];
  /** Eight percentage-offset keys (TL/TR/BR/BL x,y); invalid quads report an effect error. */
  convexQuad?: string[];
  ui?: (
    | {
        kind: "point";
        x: string;
        y: string;
        label?: string;
        advanced?: boolean;
      }
    | {
        kind: "ramp";
        start: string;
        end: string;
        midpoint?: string;
        mode?: string;
      }
  )[];
}
export interface Effects {
  register(definition: EffectDefinition): () => void;
  list(): EffectDefinition[];
  get(id: string): EffectDefinition | undefined;
}
