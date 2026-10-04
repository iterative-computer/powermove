import { randomUUID } from 'node:crypto';

import type { CodexAnswerRequest, CodexTraceEvent } from '../shared/ipc';

/** An action the Project sandbox would block, described for the person. */
export interface ApprovalRequest {
  /** What the agent wants to do, e.g. "Run outside the project sandbox". */
  title: string;
  /** The command, path or tool input, shown in mono. */
  detail: string;
  /** The agent's own reason, when it gave one. */
  reason?: string;
}

export type ApprovalDecision = { allowed: true } | { allowed: false; message: string };
export type RequestApproval = (request: ApprovalRequest, signal?: AbortSignal) => Promise<ApprovalDecision>;

export const ALLOW_ONCE = 'Allow once';
export const ALLOW_RUN = 'Allow for this run';
export const DENY = 'Deny';
const QUESTION_ID = 'decision';
/** As long as the renderer waits on a held question before giving up. */
const APPROVAL_TIMEOUT_MS = 3_600_000;
const DETAIL_CHARS = 2_000;

interface HeldApproval {
  runId: string;
  settle: (decision: ApprovalDecision) => void;
}

function fenced(text: string): string {
  const clipped = text.length > DETAIL_CHARS ? `${text.slice(0, DETAIL_CHARS)}…` : text;
  const fence = '`'.repeat(Math.max(3, ...[...clipped.matchAll(/`+/g)].map(match => match[0].length + 1)));
  return `${fence}\n${clipped}\n${fence}`;
}

/**
 * Supervised runs ask the person before anything the sandbox blocks. Each ask
 * is a held question card on the run's trace; `answer` settles it from the
 * card, and a run that ends denies whatever is still waiting.
 */
export class AgentApprovals {
  private readonly held = new Map<string, HeldApproval>();
  /** Runs where the person chose "Allow for this run". */
  private readonly allowedRuns = new Set<string>();

  requester(runId: string, onTrace: (step: CodexTraceEvent) => void): RequestApproval {
    return (request, signal) => {
      if (this.allowedRuns.has(runId)) return Promise.resolve({ allowed: true });
      if (signal?.aborted) return Promise.resolve({ allowed: false, message: 'The request was cancelled.' });
      const itemId = `approval-${randomUUID()}`;
      return new Promise<ApprovalDecision>(resolve => {
        const timer = setTimeout(() => settle({ allowed: false, message: 'Nobody answered the approval request. Continue without it.' }, true), APPROVAL_TIMEOUT_MS);
        timer.unref();
        const onAbort = () => settle({ allowed: false, message: 'The request was cancelled.' }, true);
        const settle = (decision: ApprovalDecision, close = false) => {
          if (!this.held.has(itemId)) return;
          this.held.delete(itemId);
          clearTimeout(timer);
          signal?.removeEventListener('abort', onAbort);
          if (close) onTrace({ kind: 'question-closed', itemId });
          resolve(decision);
        };
        signal?.addEventListener('abort', onAbort, { once: true });
        this.held.set(itemId, { runId, settle });
        onTrace({
          kind: 'question', itemId, transport: 'reply', blocking: true,
          questions: [{
            id: QUESTION_ID,
            header: 'Approval needed',
            question: `${request.title}\n\n${fenced(request.detail)}${request.reason ? `\n\n${request.reason.slice(0, 600)}` : ''}`,
            options: [
              { label: ALLOW_ONCE, description: '' },
              { label: ALLOW_RUN, description: 'Don’t ask again until this run ends' },
              { label: DENY, description: '' }
            ],
            allowOther: true,
            secret: false
          }]
        });
      });
    };
  }

  /** Settles a held approval from its card. A typed answer declines with that note. */
  answer(req: CodexAnswerRequest): boolean {
    const held = this.held.get(req.itemId);
    if (!held || held.runId !== req.id) return false;
    const chosen = (req.answers[QUESTION_ID] ?? []).map(answer => answer.trim()).filter(Boolean);
    if (chosen.includes(ALLOW_RUN)) this.allowedRuns.add(req.id);
    if (chosen.includes(ALLOW_ONCE) || chosen.includes(ALLOW_RUN)) {
      held.settle({ allowed: true });
      return true;
    }
    const note = chosen.filter(answer => answer !== DENY).join(', ');
    held.settle({
      allowed: false,
      message: note
        ? `The user declined and said: ${note}`
        : 'The user declined. Do not retry it; continue inside the sandbox or explain what they need to do themselves.'
    });
    return true;
  }

  /** The run ended: deny anything still waiting and forget its standing allowance. */
  closeRun(runId: string): void {
    this.allowedRuns.delete(runId);
    for (const held of [...this.held.values()]) {
      if (held.runId === runId) held.settle({ allowed: false, message: 'The run ended.' });
    }
  }
}
