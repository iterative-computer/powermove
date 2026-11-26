import type { Session } from 'better-auth/types';
import type { Data } from './db/client';
export type Env = { Bindings: CloudflareBindings; Variables: { session: Session | null; data: Data; requestId: string; /** A connection of its own, for work that outlives the response (a stream's ledger write). The caller ends it. */ openData: () => Data } };
