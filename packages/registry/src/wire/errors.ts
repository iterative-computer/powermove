import { z } from 'zod';
import type { RegistryLimitCode } from '../limits';
import type { ScanKind } from '../scan';
import { Permission, Sha1 } from './common';

export const API_ERROR_CODES = [
  'human_verification_required', 'bad_request', 'manifest_invalid', 'tree_invalid', 'handle_invalid', 'handle_reserved', 'version_invalid', 'unauthorized', 'forbidden', 'not_owner', 'not_found', 'head_moved', 'version_exists', 'handle_taken', 'handle_already_set', 'object_conflict', 'id_collision', 'same_as_origin', 'self_origin', 'author_mismatch', 'gone', 'too_large', 'object_missing', 'commit_missing', 'scan_blocked', 'permission_undeclared', 'limit_exceeded', 'rate_limited', 'quota_exceeded', 'internal', 'client_too_old'
] as const;
export type ApiErrorCode = (typeof API_ERROR_CODES)[number];
const limitCode = z.enum(['too_many_files', 'tree_too_large', 'file_too_large', 'path_too_long'] satisfies RegistryLimitCode[]);
const scanKind = z.enum(['openai_key', 'anthropic_key', 'aws_access_key', 'github_token', 'gitlab_token', 'slack_token', 'stripe_key', 'google_api_key', 'jwt', 'pem_private_key', 'high_entropy'] satisfies ScanKind[]);
const simple = <T extends ApiErrorCode>(code: T) => z.object({ error: z.literal(code), detail: z.string().optional() });
export const ApiErrorBody = z.discriminatedUnion('error', [
  simple('human_verification_required').extend({ ticket: z.string().max(2048), scope: z.string().max(128) }),
  simple('bad_request'), simple('manifest_invalid'), simple('tree_invalid'),
  simple('handle_invalid'), simple('handle_reserved'), simple('version_invalid'),
  simple('unauthorized'), simple('forbidden'), simple('not_owner'), simple('not_found'),
  simple('version_exists'), simple('handle_taken'), simple('handle_already_set'),
  simple('id_collision'), simple('same_as_origin'), simple('self_origin'),
  simple('author_mismatch'), simple('commit_missing'), simple('rate_limited'),
  simple('quota_exceeded'), simple('internal'),
  simple('head_moved').extend({ head: Sha1 }),
  simple('object_conflict').extend({ sha: Sha1 }),
  simple('gone').extend({ reason: z.enum(['removed', 'tombstoned', 'yanked']) }),
  simple('too_large').extend({ limit: z.union([limitCode, z.literal('envelope')]) }),
  simple('object_missing').extend({ shas: z.array(Sha1) }),
  simple('scan_blocked').extend({ findings: z.array(z.object({ path: z.string(), line: z.number().int().positive(), kind: scanKind })) }),
  simple('permission_undeclared').extend({ findings: z.array(z.object({ path: z.string(), line: z.number().int().positive(), capability: Permission.extract(['network', 'clipboard']) })) }),
  simple('limit_exceeded').extend({ code: limitCode }),
  simple('client_too_old').extend({ minimum: z.string() })
]);
export type ApiErrorBody = z.infer<typeof ApiErrorBody>;
export const API_ERROR_STATUS: Record<ApiErrorCode, number> = {
  human_verification_required: 403, bad_request: 400, manifest_invalid: 400, tree_invalid: 400, handle_invalid: 400, handle_reserved: 400, version_invalid: 400,
  unauthorized: 401, forbidden: 403, not_owner: 403, not_found: 404,
  head_moved: 409, version_exists: 409, handle_taken: 409, handle_already_set: 409, object_conflict: 409,
  id_collision: 409, same_as_origin: 409, self_origin: 409, author_mismatch: 409,
  gone: 410, too_large: 413, object_missing: 422, commit_missing: 422, scan_blocked: 422, permission_undeclared: 422, limit_exceeded: 422,
  rate_limited: 429, quota_exceeded: 429, internal: 500, client_too_old: 426
} satisfies Record<ApiErrorCode, number>;
export class ApiError extends Error {
  constructor(public readonly body: ApiErrorBody) { super(body.detail ?? body.error); this.name = 'ApiError'; }
  get status(): number { return API_ERROR_STATUS[this.body.error]; }
}
