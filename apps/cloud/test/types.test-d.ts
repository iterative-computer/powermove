import type { AppType } from '../src/index';
import { hc, type InferResponseType } from 'hono/client';
declare const client: ReturnType<typeof hc<AppType>>;
type Get = typeof client.v1.me.$get;
type Response = InferResponseType<Get, 200>;
type Expect<T extends true> = T;
type _HasUser = Expect<Response extends { user: { id: string; email: string }; publisher: unknown; settings: { rememberInstalls: boolean } } ? true : false>;
