export const EDIT_COMMAND_TYPES = [
  'set_property',
  'replace_keyframes',
  'set_easing',
  'set_expression',
  'set_content',
  'set_layer',
  'set_composition',
  'add_layer',
  'delete_layers',
  'reorder_layer',
  'group_layers',
  'ungroup_layers',
  'move_to_group',
  'add_effect',
  'remove_effect',
  'set_effect',
  'set_transition',
  'set_scene_parameter',
  'add_marker',
  'create_section',
  'update_section',
  'transform_layers'
] as const;

// Every source-edit operation is currently reachable by the production agent.
// Kept as its OWN tuple (not an alias) so adding a future non-agent-safe op to
// EDIT_COMMAND_TYPES forces a deliberate decision here — and fails the
// vocabulary contract test until one is made.
export const AGENT_COMMAND_TYPES = [
  'set_property',
  'replace_keyframes',
  'set_easing',
  'set_expression',
  'set_content',
  'set_layer',
  'set_composition',
  'add_layer',
  'delete_layers',
  'reorder_layer',
  'group_layers',
  'ungroup_layers',
  'move_to_group',
  'add_effect',
  'remove_effect',
  'set_effect',
  'set_transition',
  'set_scene_parameter',
  'add_marker',
  'create_section',
  'update_section',
  'transform_layers'
] as const;

export type EditCommandType = (typeof EDIT_COMMAND_TYPES)[number];
