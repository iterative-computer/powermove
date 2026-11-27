/* v2 errors carry a code and nothing else: `{"error": {"code": "..."}}`. The backend's own error text never
   reaches a v2 response body (app.ts maps v1 ApiErrors and unknown failures under /v2 to codes too). */
export class V2Error extends Error {
  constructor(readonly status: 400 | 401 | 402 | 403 | 404 | 409 | 413 | 429 | 500 | 502 | 503, readonly code: string, readonly retryAfter?: number) { super(code); }
}
