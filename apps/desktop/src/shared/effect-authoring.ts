/** The meaning of an effect request is the same in every agent mode. */
export const EFFECT_AUTHORING_INSTRUCTIONS = `EFFECT REQUESTS
Effects & Presets: reuse definitions or api.effects.register with keyframeable params before add_effect. backdrop: true reads u_backdrop. Offsets: u_effectScale * u_texel; noise: effectPixel(), never screen coordinates. Declare viewportPadding before viewportSafe; preview/zoom must match export. Use families (Blur & Sharpen, Light & Shadow, Color, Stylize, Distort, Generate); new only if none fits, never effect/project/style names or duplicates. No panel/script-button/rig substitutes. Panels need a separate request; design/placement cannot change it.`;

export const EDITOR_EXTENSION_INSTRUCTIONS = `EDITOR CAPABILITIES
Composition only can apply existing effects but cannot write extension code. Use existing capabilities and normal editable commands first. If a genuinely new effect definition or extension change is needed, return operation=requires_project with neutral edit fields and explain that Auto or Supervised enables authoring. Do not approximate the deliverable with a panel or scene rig, and do not claim new functionality was created.`;
