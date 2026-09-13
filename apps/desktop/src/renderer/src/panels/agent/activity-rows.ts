/* The agent's thinking trail, collapsed into readable rows.

   A provider stream emits one event per tool call. Rendering them one-to-one
   produces a stuttering list of near-identical lines ("Read file", "Read file",
   "Read file"). Instead, consecutive tool steps collapse into ONE row with a
   natural-language label — "Ran commands, edited files, and searched the web"
   — and thinking/text steps stay where they are so the trail keeps its
   chronology. Ported from supermove. */

export type { TraceStep } from './agent-state.svelte';
import type { TraceStep } from './agent-state.svelte';

export type ToolsRowStatus = 'running' | 'partial' | 'error' | 'done';

/** One call inside a tool group — everything the expandable work log shows. */
export type ToolDetail = Pick<ToolStep, 'id' | 'label' | 'status'> &
  Partial<Pick<ToolStep, 'toolName' | 'detail' | 'output' | 'startedAt' | 'endedAt'>> & {
    family: ToolFamily;
  };

export interface ToolsRow {
  kind: 'tools';
  id: string;
  label: string;
  status: ToolsRowStatus;
  failedCount: number;
  successCount: number;
  toolCount: number;
  /** The provider-emitted label for the tool doing work right now. */
  currentLabel?: string;
  /** The mono argument of the tool doing work right now (command, path, query). */
  currentDetail?: string;
  /** Real provider-emitted activity, retained for the expandable work log. */
  details: ToolDetail[];
  /** Individual step labels, kept for a title tooltip on settled groups. */
  detail?: string[];
  /** Wall-clock span of the group, when the provider stamped its calls. */
  startedAt?: number;
  endedAt?: number;
}

/* Icon family for a tool row. Coarser than `toolAction`: the glyph only needs
   to say "shell", "file", "search" — the label says the rest. */
export type ToolFamily = 'run' | 'edit' | 'read' | 'search' | 'image' | 'computer' | 'panel' | 'think' | 'tool';

const FAMILY_RULES: Array<[RegExp, ToolFamily]> = [
  [/panel|workspace|project_state|render_frames|apply_commands|edit_video|rollback/, 'panel'],
  [/^(web|search|browse|fetch|grep|glob|find|ls$|list)|url/, 'search'],
  [/^(bash|command|shell|terminal|exec|run|process)/, 'run'],
  [/^(edit|write|create|patch|apply|delete|move|multiedit|notebook)|file_change/, 'edit'],
  [/^(read|view|cat)/, 'read'],
  [/^(image|render|draw|screenshot)/, 'image'],
  [/^computer/, 'computer'],
  [/^(agent|task|todo|plan)/, 'think']
];

export function toolFamily(toolName?: string): ToolFamily {
  const name = String(toolName ?? '').trim().toLowerCase();
  if (!name) return 'tool';
  const short = name.startsWith('mcp__') ? (name.split('__').at(-1) || name) : name;
  for (const [pattern, family] of FAMILY_RULES) {
    if (pattern.test(short)) return family;
  }
  return 'tool';
}

/* "4s", "1m 12s" — the settled-header duration, harness style. Sub-second
   work reads as "<1s" rather than "0s", which looks like a failure. */
export function durationLabel(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '';
  const seconds = Math.round(ms / 1000);
  if (seconds < 1) return '<1s';
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
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
  [/^get_project_state$/, 'inspected the project'],
  [/^get_panel_layout$/, 'checked the panel layout'],
  [/^open_panel$/, 'opened a panel'],
  [/^get_panel_state$/, 'checked panel controls'],
  [/^interact_panel$/, 'used panel controls'],
  [/^capture_panel$/, 'captured a panel'],
  [/^computer_use_panel$/, 'tested panel controls'],
  [/^get_workspace_state$/, 'inspected the workspace'],
  [/^render_frames$/, 'previewed composition frames'],
  [/^apply_commands$/, 'edited the composition'],
  [/^edit_video$/, 'edited video clips'],
  [/^rollback_changes$/, 'undid agent changes'],
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
  // Strip provider namespaces before matching so native and MCP calls agree.
  const short = name.startsWith('mcp__') ? (name.split('__').at(-1) || name) : name;
  for (const [pattern, action] of ACTION_RULES) {
    if (pattern.test(short)) return action;
  }
  return `used ${short.replace(/[_-]+/g, ' ')}`;
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
  const failedCount = steps.filter((step) => step.status === 'error').length;
  const successCount = steps.filter((step) => step.status === 'done').length;
  const settledCount = steps.filter((step) => step.status !== 'running').length;
  // `continued` is a bookkeeping state (the call was resumed), not a failure —
  // it stops pulsing like `done`, but is not counted as confirmed success.
  // A failed exploratory check must not paint successful
  // commands and edits in the same batch as wholly failed.
  const status: ToolsRowStatus = steps.some((step) => step.status === 'running')
    ? 'running'
    : failedCount === settledCount && failedCount > 0
      ? 'error'
      : failedCount > 0
        ? 'partial'
      : 'done';
  const current = [...steps].reverse().find((step) => step.status === 'running' && step.label);
  const started = steps.map((step) => step.startedAt).filter((value): value is number => typeof value === 'number');
  const ended = steps.map((step) => step.endedAt).filter((value): value is number => typeof value === 'number');
  const row: ToolsRow = {
    kind: 'tools',
    id: `tools-${steps[0]?.id ?? steps[0]?.toolName ?? 'activity'}`,
    label: label.charAt(0).toUpperCase() + label.slice(1),
    status,
    failedCount,
    successCount,
    toolCount: steps.length,
    currentLabel: current?.label,
    currentDetail: current?.detail,
    details: steps
      .filter((step) => Boolean(step.label))
      .map(({ id, label, status, toolName, detail, output, startedAt, endedAt }) => ({
        id, label, status, toolName, detail, output, startedAt, endedAt, family: toolFamily(toolName)
      }))
  };
  if (started.length) row.startedAt = Math.min(...started);
  // A settled group's end is its last completed call; a running group has none yet.
  if (status !== 'running' && ended.length === settledCount && ended.length) row.endedAt = Math.max(...ended);
  // Powermove keeps what supermove drops: once a group settles, the individual
  // labels stay reachable as a tooltip. Detail without visual noise.
  if (status !== 'running' && steps.length > 1) {
    row.detail = steps
      .filter((step) => Boolean(step.label))
      .map((step) => failedCount > 0
        ? `${step.status === 'error' ? 'Failed' : step.status === 'continued' ? 'Continued' : 'Succeeded'} · ${step.label}`
        : step.label);
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
