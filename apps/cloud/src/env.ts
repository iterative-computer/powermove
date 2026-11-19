import type { Session } from 'better-auth/types';
import type { Data } from './db/client';
export type Env = { Bindings: CloudflareBindings; Variables: { session: Session | null; data: Data; requestId: string; /** A new connection for work that outlives the request, such as metering a stream. Caller ends it. */ freshData: () => Data } };
