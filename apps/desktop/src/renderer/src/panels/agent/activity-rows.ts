/* The agent's thinking trail, collapsed into readable rows.

   A provider stream emits one event per tool call. Rendering them one-to-one
   produces a stuttering list of near-identical lines ("Read file", "Read file",
   "Read file"). Instead, consecutive tool steps collapse into ONE row with a
   natural-language label — "Ran commands, edited files, and searched the web"
   — reasoning folds into the same group as rows of their own, and text stays
   where it is so the trail keeps its chronology. Ported from supermove, laid
   out after the beautiful-ui harness. */

export type { TraceStep } from './agent-state.svelte';
import type { TraceStep } from './agent-state.svelte';

export type ToolsRowStatus = 'running' | 'partial' | 'error' | 'done';

/** One row inside a work group: a tool call, or a stretch of reasoning.
    Thoughts ride in the same list as calls (harness ToolChips idiom): label
    "Thinking", the first sentence as the chip, the full text as the output. */
export type ToolDetail = Pick<ToolStep, 'id' | 'label' | 'status'> &
  Partial<Pick<ToolStep, 'toolName' | 'detail' | 'output' | 'startedAt' | 'endedAt'>> & {
    kind: 'tool' | 'thought';
    family: ToolFamily;
  };

export interface ToolsRow {
  kind: 'tools';
  id: string;
  /** Header text: "4 tool calls", or "Thinking" / "Thought for 4s" for reasoning alone. */
  label: string;
  /** What was done, in words: "Ran commands and edited files". */
  summary: string;
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
  /** Files touched by edit-family calls, for the trailing file chips. */
  files: string[];
  /** Number of reasoning stretches folded into this group. */
  thoughtCount: number;
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
type ThoughtStep = Extract<TraceStep, { kind: 'thought' }>;
type WorkStep = ToolStep | ThoughtStep;

/* The chip for a thought is its opening: first sentence, one line. */
export function thoughtLead(text: string): string {
  const flat = String(text ?? '').replace(/\*\*/g, '').replace(/\s+/g, ' ').trim();
  const sentence = flat.match(/^.{12,}?[.!?](\s|$)/)?.[0]?.trim();
  const lead = sentence && sentence.length < flat.length ? sentence : flat;
  return lead.length > 96 ? `${lead.slice(0, 95).trimEnd()}…` : lead;
}

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

function thoughtDetail(step: ThoughtStep): ToolDetail {
  const span = step.startedAt !== undefined && step.endedAt !== undefined ? durationLabel(step.endedAt - step.startedAt) : '';
  return {
    kind: 'thought',
    id: step.id,
    label: step.live ? 'Thinking' : span ? `Thought for ${span}` : 'Thought',
    status: step.live ? 'running' : 'done',
    detail: thoughtLead(step.label),
    output: step.label,
    startedAt: step.startedAt,
    endedAt: step.endedAt,
    family: 'think'
  };
}

export function groupedToolRow(work: WorkStep[]): ToolsRow {
  const steps = work.filter((step): step is ToolStep => step.kind === 'tool');
  const thoughts = work.filter((step): step is ThoughtStep => step.kind === 'thought');
  const actions = [...new Set(steps.map((step) => toolAction(step.toolName)))];
  const failedCount = steps.filter((step) => step.status === 'error').length;
  const successCount = steps.filter((step) => step.status === 'done').length;
  const settledCount = steps.filter((step) => step.status !== 'running').length;
  const thinking = thoughts.some((step) => step.live);
  // `continued` is a bookkeeping state (the call was resumed), not a failure —
  // it stops pulsing like `done`, but is not counted as confirmed success.
  // A failed exploratory check must not paint successful
  // commands and edits in the same batch as wholly failed.
  const status: ToolsRowStatus = steps.some((step) => step.status === 'running') || thinking
    ? 'running'
    : failedCount === settledCount && failedCount > 0
      ? 'error'
      : failedCount > 0
        ? 'partial'
      : 'done';
  const started = work.map((step) => step.startedAt).filter((value): value is number => typeof value === 'number');
  const ended = work.map((step) => step.endedAt).filter((value): value is number => typeof value === 'number');
  const span = started.length && ended.length && status !== 'running' ? durationLabel(Math.max(...ended) - Math.min(...started)) : '';
  // The header counts calls (harness "4 tool calls"); the natural-language
  // summary of what was done stays reachable as the tooltip. A reasoning-only
  // stretch reads like the harness ThinkingState header.
  const label = steps.length
    ? `${steps.length} tool ${steps.length === 1 ? 'call' : 'calls'}`
    : thinking ? 'Thinking' : span ? `Thought for ${span}` : 'Thought';
  const current = [...steps].reverse().find((step) => step.status === 'running' && step.label);
  const row: ToolsRow = {
    kind: 'tools',
    id: `tools-${work[0]?.id ?? 'activity'}`,
    label,
    summary: steps.length ? joinActions(actions).replace(/^./, (c) => c.toUpperCase()) : label,
    status,
    failedCount,
    successCount,
    toolCount: steps.length,
    thoughtCount: thoughts.length,
    currentLabel: current?.label,
    currentDetail: current?.detail,
    details: work.map((step) => step.kind === 'thought'
      ? thoughtDetail(step)
      : {
          kind: 'tool' as const,
          id: step.id, label: step.label, status: step.status, toolName: step.toolName,
          detail: step.detail, output: step.output, startedAt: step.startedAt, endedAt: step.endedAt,
          family: toolFamily(step.toolName)
        }).filter((detail) => Boolean(detail.label)),
    files: [...new Set(steps
      .filter((step) => toolFamily(step.toolName) === 'edit' && step.detail && step.status !== 'error')
      .flatMap((step) => String(step.detail).split(/,\s*/).map((part) => basename(part.trim())).filter(Boolean)))]
  };
  if (started.length) row.startedAt = Math.min(...started);
  // A settled group's end is its last completed step; a running group has none yet.
  if (status !== 'running' && ended.length) row.endedAt = Math.max(...ended);
  // Powermove keeps what supermove drops: once a group settles, the individual
  // labels stay reachable as a tooltip. Detail without visual noise.
  if (status !== 'running' && steps.length > 1) {
    row.detail = [row.summary, ...steps
      .filter((step) => Boolean(step.label))
      .map((step) => failedCount > 0
        ? `${step.status === 'error' ? 'Failed' : step.status === 'continued' ? 'Continued' : 'Succeeded'} · ${step.label}`
        : step.label)];
  }
  return row;
}

export function activityRows(steps: TraceStep[] = []): ActivityRow[] {
  const rows: Array<TraceStep | ToolsRow> = [];
  let work: WorkStep[] = [];
  const flushWork = (): void => {
    if (!work.length) return;
    rows.push(groupedToolRow(work));
    work = [];
  };

  // Reasoning and calls share one group; only model prose breaks it, so the
  // trail reads text → work → text in stream order.
  for (const step of steps) {
    if (step?.kind === 'tool' || step?.kind === 'thought') {
      work.push(step);
    } else {
      flushWork();
      if (step?.kind === 'text') rows.push(step);
    }
  }
  flushWork();

  // Replayed or interleaved provider events can leave more than one historical
  // row marked live/running. Preserve those states for diagnostics, but give
  // the motion treatment to only the newest active row so the activity trail
  // always has one clear point of focus.
  let pulsingIndex = -1;
  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index] as any;
    if (row.kind === 'tools' && row.status === 'running') {
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
