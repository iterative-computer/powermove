import { z } from 'zod';

export const orchestrationTarget = z.object({
  providerInstanceId: z.enum(['chatgpt', 'claude', 'compatible']).optional(),
  model: z.string().min(1).max(200).optional(),
  options: z.object({ reasoningEffort: z.enum(['none', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra']).optional() }).strict().optional()
}).strict();
const id = z.string().min(1).max(120).regex(/^[A-Za-z0-9_-]+$/);
export const threadInput = {
  thread_launch: z.object({ title: z.string().trim().min(1).max(64), message: z.string().trim().min(1).max(50_000), target: orchestrationTarget.optional(), clientRequestId: id.optional() }).strict(),
  thread_list: z.object({ offset: z.number().int().min(0).default(0), limit: z.number().int().min(1).max(100).default(30) }).strict(),
  thread_read: z.object({ threadId: id, offset: z.number().int().min(0).default(0), limit: z.number().int().min(1).max(100).default(20), textOffset: z.number().int().min(0).default(0), maxCharsPerMessage: z.number().int().min(1).max(20_000).default(12_000) }).strict(),
  thread_send: z.object({ threadId: id, message: z.string().trim().min(1).max(50_000), target: orchestrationTarget.optional(), clientRequestId: id.optional() }).strict(),
  thread_watch: z.object({ threadId: id, runId: id.optional(), mode: z.enum(['async', 'wait']).default('async'), timeoutMs: z.number().int().min(0).max(90_000).default(90_000) }).strict(),
  thread_unwatch: z.object({ threadId: id, runId: id.optional() }).strict(),
  thread_cancel: z.object({ threadId: id }).strict(),
};
