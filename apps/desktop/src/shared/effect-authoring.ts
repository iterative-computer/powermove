/** The meaning of an effect request is the same in every agent mode. */
export const EFFECT_AUTHORING_INSTRUCTIONS = `EFFECT REQUESTS
A requested effect belongs in Effects & Presets: use an existing definition or author one with api.effects.register and keyframeable params. Set backdrop: true to sample beneath the layer via u_backdrop. add_effect only applies an already registered definition. Never substitute a panel, script button, or layer rig for a requested effect. Create panels only when the user requests a panel or a separate workflow needs one; panel design and placement instructions do not change the requested deliverable.`;

export const EDITOR_EXTENSION_INSTRUCTIONS = `EDITOR CAPABILITIES
This mode can apply existing effects but cannot author or modify extension code. When the request needs a new effect definition or another extension change, return operation=requires_project with neutral edit fields. Explain the needed capability in message. Powermove will offer Project access to continue the original request. Do not approximate the request with a panel or scene rig, and do not claim that new functionality was created.`;
