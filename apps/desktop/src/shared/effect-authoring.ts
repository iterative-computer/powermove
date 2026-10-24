import { DEPRECATION_GUIDANCE } from './extension-analysis';

/** The meaning of an effect request is the same in every agent mode. */
export const EFFECT_AUTHORING_INSTRUCTIONS = `EFFECT REQUESTS
Effects belong in Effects & Presets: use existing definitions or api.effects.register with keyframeable params. backdrop: true samples beneath via u_backdrop. add_effect requires registration. Choose an existing group (Blur & Sharpen, Light & Shadow, Color, Stylize, Distort, Generate) or another listed family. Create a group only if none fits; never name it after the effect/project/style or duplicate a family. Never substitute panels, script buttons or layer rigs. Panels require a request/separate workflow; design/placement guidance never changes the deliverable.
${DEPRECATION_GUIDANCE}`;

export const EDITOR_EXTENSION_INSTRUCTIONS = `EDITOR CAPABILITIES
Composition only can apply existing effects but cannot write extension code. Use existing capabilities and normal editable commands first. If a genuinely new effect definition or extension change is needed, return operation=requires_project with neutral edit fields and explain that Auto or Supervised enables authoring. Do not approximate the deliverable with a panel or scene rig, and do not claim new functionality was created.`;
