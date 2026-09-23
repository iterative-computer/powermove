import { z } from 'zod';
import type { RegistryLimitCode } from '../limits';
import type { ScanKind } from '../scan';
import { Sha1 } from './common';

export enum ApiErrorCode {
  bad_request = 'bad_request', manifest_invalid = 'manifest_invalid', tree_invalid = 'tree_invalid',
  handle_invalid = 'handle_invalid', handle_reserved = 'handle_reserved', version_invalid = 'version_invalid',
  unauthorized = 'unauthorized', forbidden = 'forbidden', not_owner = 'not_owner', not_found = 'not_found',
  head_moved = 'head_moved', version_exists = 'version_exists', handle_taken = 'handle_taken',
  handle_already_set = 'handle_already_set', object_conflict = 'object_conflict', id_collision = 'id_collision',
  same_as_origin = 'same_as_origin', self_origin = 'self_origin', author_mismatch = 'author_mismatch',
  gone = 'gone', too_large = 'too_large', object_missing = 'object_missing', commit_missing = 'commit_missing',
  scan_blocked = 'scan_blocked', limit_exceeded = 'limit_exceeded', rate_limited = 'rate_limited',
  quota_exceeded = 'quota_exceeded', internal = 'internal', client_too_old = 'client_too_old'
}
const limitCode = z.enum(['too_many_files', 'tree_too_large', 'file_too_large', 'path_too_long'] satisfies RegistryLimitCode[]);
const scanKind = z.enum(['openai_key', 'anthropic_key', 'aws_access_key', 'github_token', 'gitlab_token', 'slack_token', 'stripe_key', 'google_api_key', 'jwt', 'pem_private_key', 'high_entropy'] satisfies ScanKind[]);
const simple = <T extends ApiErrorCode>(code: T) => z.object({ error: z.literal(code), detail: z.string().optional() });
export const ApiErrorBody = z.discriminatedUnion('error', [
  simple(ApiErrorCode.bad_request), simple(ApiErrorCode.manifest_invalid), simple(ApiErrorCode.tree_invalid),
  simple(ApiErrorCode.handle_invalid), simple(ApiErrorCode.handle_reserved), simple(ApiErrorCode.version_invalid),
  simple(ApiErrorCode.unauthorized), simple(ApiErrorCode.forbidden), simple(ApiErrorCode.not_owner), simple(ApiErrorCode.not_found),
  simple(ApiErrorCode.version_exists), simple(ApiErrorCode.handle_taken), simple(ApiErrorCode.handle_already_set),
  simple(ApiErrorCode.id_collision), simple(ApiErrorCode.same_as_origin), simple(ApiErrorCode.self_origin),
  simple(ApiErrorCode.author_mismatch), simple(ApiErrorCode.commit_missing), simple(ApiErrorCode.rate_limited),
  simple(ApiErrorCode.quota_exceeded), simple(ApiErrorCode.internal),
  simple(ApiErrorCode.head_moved).extend({ head: Sha1 }),
  simple(ApiErrorCode.object_conflict).extend({ sha: Sha1 }),
  simple(ApiErrorCode.gone).extend({ reason: z.enum(['removed', 'tombstoned', 'yanked']) }),
  simple(ApiErrorCode.too_large).extend({ limit: z.union([limitCode, z.literal('envelope')]) }),
  simple(ApiErrorCode.object_missing).extend({ shas: z.array(Sha1) }),
  simple(ApiErrorCode.scan_blocked).extend({ findings: z.array(z.object({ path: z.string(), line: z.number().int().positive(), kind: scanKind })) }),
  simple(ApiErrorCode.limit_exceeded).extend({ code: limitCode }),
  simple(ApiErrorCode.client_too_old).extend({ minimum: z.string() })
]);
export type ApiErrorBody = z.infer<typeof ApiErrorBody>;
export const API_ERROR_STATUS: Record<ApiErrorCode, number> = {
  bad_request: 400, manifest_invalid: 400, tree_invalid: 400, handle_invalid: 400, handle_reserved: 400, version_invalid: 400,
  unauthorized: 401, forbidden: 403, not_owner: 403, not_found: 404,
  head_moved: 409, version_exists: 409, handle_taken: 409, handle_already_set: 409, object_conflict: 409,
  id_collision: 409, same_as_origin: 409, self_origin: 409, author_mismatch: 409,
  gone: 410, too_large: 413, object_missing: 422, commit_missing: 422, scan_blocked: 422, limit_exceeded: 422,
  rate_limited: 429, quota_exceeded: 429, internal: 500, client_too_old: 426
} satisfies Record<ApiErrorCode, number>;
export class ApiError extends Error {
  constructor(public readonly body: ApiErrorBody) { super(body.detail ?? body.error); this.name = 'ApiError'; }
  get status(): number { return API_ERROR_STATUS[this.body.error]; }
}
