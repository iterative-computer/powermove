export interface NativeMcpServerConfig {
  command: string;
  args: string[];
  env: Record<string, string>;
}

export interface PowermoveAgentToolSpec {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

const closedObject = (properties: Record<string, unknown>, required: string[] = []): Record<string, unknown> => ({
  type: 'object',
  additionalProperties: false,
  properties,
  ...(required.length ? { required } : {})
});

export const POWERMOVE_AGENT_TOOLS: readonly PowermoveAgentToolSpec[] = [
  {
    name: 'get_project_state',
    description: 'Read the live Powermove composition, selection, layers, editable properties, keyframes, effects, markers, and current revision. Call this again after edits instead of assuming cached state.',
    inputSchema: closedObject({})
  },
  {
    name: 'get_panel_layout',
    description: 'Read the live Powermove dock and panel layout, including registered panel ids and titles. Use this before deciding where an interface change belongs.',
    inputSchema: closedObject({})
  },
  {
    name: 'render_frames',
    description: 'Render one to five frames from the live composition. Returns the chosen times plus real PNG or JPEG images for visual review.',
    inputSchema: closedObject({
      times: {
        type: 'array',
        maxItems: 5,
        items: { type: 'number', minimum: 0 }
      },
      width: { type: 'integer', minimum: 160, maximum: 1280 }
    })
  },
  {
    name: 'apply_commands',
    description: 'Apply one to eighty typed Powermove edit commands to the live composition as a guarded transaction. Commands remain editable and keyframeable and are grouped into one Undo for the whole agent run. Never edit the project JSON directly.',
    inputSchema: closedObject({
      label: { type: 'string', minLength: 1, maxLength: 80 },
      commands: {
        type: 'array',
        minItems: 1,
        maxItems: 80,
        items: {
          oneOf: [
            { type: 'string', maxLength: 50_000 },
            { type: 'object' }
          ]
        }
      }
    }, ['commands'])
  },
  {
    name: 'edit_video',
    description: 'Edit real video/audio timeline clips. Operations: insert an existing imported asset; split at a composition time (returns tailId); trim to a retained composition-time range; move; change constant video speed while preserving the source range; remove without ripple; separate_audio (returns audioId); set audio gain/fades. Times are seconds. Read get_project_state for layer and media asset IDs. Repeated calls share one Undo with apply_commands. Animated timing requires explicit property/keyframe commands. Use render_frames to review cuts.',
    inputSchema: closedObject({
      operation: { type: 'string', enum: ['insert', 'split', 'trim', 'move', 'speed', 'remove', 'separate_audio', 'audio'] },
      layerId: { type: 'string' },
      assetId: { type: 'string' },
      label: { type: 'string', maxLength: 80 },
      at: { type: 'number', minimum: 0 },
      start: { type: 'number', minimum: 0 },
      end: { type: 'number', minimum: 0 },
      from: { type: 'number', minimum: 0 },
      sourceIn: { type: 'number', minimum: 0 },
      duration: { type: 'number', exclusiveMinimum: 0 },
      speed: { type: 'number', minimum: 0.01 },
      gain: { type: 'number', minimum: 0 },
      fadeIn: { type: 'number', minimum: 0 },
      fadeOut: { type: 'number', minimum: 0 }
    }, ['operation'])
  },
  {
    name: 'rollback_changes',
    description: 'Roll back every live composition edit made by this agent run, without touching earlier project work. The run may make a fresh guarded edit after rolling back.',
    inputSchema: closedObject({})
  }
] as const;

export const POWERMOVE_MCP_TOOL_NAMES = POWERMOVE_AGENT_TOOLS.map((tool) =>
  `mcp__powermove__${tool.name}`);
