/* The agent's thinking trail, collapsed into readable rows.

   A provider stream emits one event per tool call. Rendering them one-to-one
   produces a stuttering list of near-identical lines ("Read file", "Read file",
   "Read file"). Instead, consecutive tool steps collapse into ONE row with a
   natural-language label — "Ran commands, edited files, and searched the web"
   — and thinking/text steps stay where they are so the trail keeps its
   chronology. Ported from supermove. */

export type TraceStep =
  | { kind: 'thought'; id: string; label: string; live: boolean }
  | { kind: 'text'; id: string; text: string }
  | {
      kind: 'tool';
      id: string;
      toolName: string;
      label: string;
      status: 'running' | 'done' | 'error' | 'continued';
    };

export type ToolsRowStatus = 'running' | 'error' | 'done';

export interface ToolsRow {
  kind: 'tools';
  id: string;
  label: string;
  status: ToolsRowStatus;
  /** Individual step labels, kept for a title tooltip on settled groups. */
  detail?: string[];
}

type RowMeta = { renderKey: string; pulsing: boolean };

export type ActivityRow =
  | (Extract<TraceStep, { kind: 'thought' }> & RowMeta)
  | (Extract<TraceStep, { kind: 'text' }> & RowMeta)
  | (ToolsRow & RowMeta);

/* Tool names come from wildly different providers (`bash`, `str_replace_edit`,
   `mcp__github__search_issues`). Match on the FAMILY, not the exact name, so a
   new tool still produces a sentence instead of a raw identifier. */
const ACTION_RULES: Array<[RegExp, string]> = [
  [/^(web|search|browse|fetch)|url/, 'searched the web'],
  [/^(bash|command|shell|terminal|exec|run|process)/, 'ran commands'],
  [/^(edit|write|create|patch|file|apply|delete|move)/, 'edited files'],
  [/^(read|grep|glob|ls|list|find|view)/, 'read files'],
  [/^(image|render|draw|screenshot)/, 'generated images'],
  [/^computer/, 'operated the computer']
];

export function toolAction(toolName?: string): string {
  const name = String(toolName ?? '').trim().toLowerCase();
  if (!name) return 'used a tool';
  for (const [pattern, action] of ACTION_RULES) {
    if (pattern.test(name)) return action;
  }
  // `mcp__server__tool` reads as noise; the last segment is the useful part.
  const short = name.startsWith('mcp') ? (name.split('__').at(-1) || name) : name;
  return `used ${short}`;
}

export function joinActions(actions: string[]): string {
  if (actions.length === 0) return 'used a tool';
  if (actions.length === 1) return actions[0] ?? 'used a tool';
  if (actions.length === 2) return `${actions[0]} and ${actions[1]}`;
  return `${actions.slice(0, -1).join(', ')}, and ${actions.at(-1)}`;
}

type ToolStep = Extract<TraceStep, { kind: 'tool' }>;

export function groupedToolRow(steps: ToolStep[]): ToolsRow {
  const actions = [...new Set(steps.map((step) => toolAction(step.toolName)))];
  const label = joinActions(actions);
  // `continued` is a bookkeeping state (the call was resumed), not a failure —
  // it settles like `done`. Running wins over error wins over done.
  const status: ToolsRowStatus = steps.some((step) => step.status === 'running')
    ? 'running'
    : steps.some((step) => step.status === 'error')
      ? 'error'
      : 'done';
  const row: ToolsRow = {
    kind: 'tools',
    id: `tools-${steps[0]?.id ?? steps[0]?.toolName ?? 'activity'}`,
    label: label.charAt(0).toUpperCase() + label.slice(1),
    status
  };
  // Powermove keeps what supermove drops: once a group settles, the individual
  // labels stay reachable as a tooltip. Detail without visual noise.
  if (status !== 'running' && steps.length > 1) {
    row.detail = steps.map((step) => step.label).filter(Boolean);
  }
  return row;
}

export function activityRows(steps: TraceStep[] = []): ActivityRow[] {
  const rows: Array<TraceStep | ToolsRow> = [];
  let tools: ToolStep[] = [];
  const flushTools = (): void => {
    if (!tools.length) return;
    rows.push(groupedToolRow(tools));
    tools = [];
  };

  for (const step of steps) {
    if (step?.kind === 'tool') {
      tools.push(step);
    } else {
      flushTools();
      if (step?.kind === 'thought' || step?.kind === 'text') rows.push(step);
    }
  }
  flushTools();

  // Replayed or interleaved provider events can leave more than one historical
  // row marked live/running. Preserve those states for diagnostics, but give
  // the motion treatment to only the newest active row so the activity trail
  // always has one clear point of focus.
  let pulsingIndex = -1;
  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index] as any;
    if ((row.kind === 'thought' && row.live) || (row.kind === 'tools' && row.status === 'running')) {
      pulsingIndex = index;
    }
  }

  // Provider streams may replay an event while reconnecting. The semantic id
  // is still useful for grouping, but it cannot safely be the Svelte each key:
  // two replayed tool/text events with the same id would otherwise take down
  // the whole renderer with `each_key_duplicate`. Include the row position so
  // every render key is unique and remains stable as the stream appends.
  return rows.map((row, index) => ({
    ...(row as any),
    renderKey: `${(row as any).id ?? (row as any).kind ?? 'activity'}-${index}`,
    pulsing: index === pulsingIndex
  })) as ActivityRow[];
}
