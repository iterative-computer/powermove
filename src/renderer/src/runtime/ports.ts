/*
 * RuntimePorts — the ONLY seam between the non-reactive core/engine world and
 * the Svelte state layer. core/ and engine/ never import a *.svelte.ts module;
 * they call these functions, installed once at the composition root.
 *
 * Invariants:
 *  - getProject() returns a plain object graph (never a $state proxy). The
 *    compositor and expression evaluator read it 60×/sec with zero signal reads.
 *  - replaceProject() is the one restore path (undo, load, Edit rollback).
 *  - invalidate() is synchronous: caches die in the same call that reports the
 *    mutation, so a paused frame can never observe stale memoized matrices.
 */
import type { Project } from '../core/types/project';

export type MutationKind =
  | 'values' // any channel/keyframe/content change → inspector value sync
  | 'structure' // add/remove/reorder/rename/timing → layer lists, timeline gutter
  | 'project' // composition settings, markers, params
  | 'assets'
  | 'library'
  | 'history'
  | 'replace'; // whole-project swap: bumps every category

export interface Selection {
  layers: string[];
  keys: string[]; // keyframe ids (kf.i)
  chan: string | null;
}

/** Plain dirty flags polled by the rAF loop — never runes. */
export interface FrameFlags {
  render: boolean;
  timeline: boolean;
  status: boolean;
}

export interface RuntimePorts {
  getProject(): Project;
  replaceProject(next: Project): void;
  getSelection(): Selection;
  setSelection(next: Selection): void;
  getTime(): number;
  setTime(t: number): void;
  invalidate(kind: MutationKind): void;
  readonly frame: FrameFlags;
}

let installed: RuntimePorts | null = null;

export function installPorts(ports: RuntimePorts): void {
  if (installed) throw new Error('RuntimePorts already installed');
  installed = ports;
}

export function ports(): RuntimePorts {
  if (!installed) throw new Error('RuntimePorts not installed — call installPorts() at the composition root first');
  return installed;
}

/** Test seam: replace or clear the installed ports. */
export function resetPortsForTests(next: RuntimePorts | null = null): void {
  installed = next;
}
