import type { Session } from 'better-auth/types';
import type { Data } from './db/client';
export type Env = { Bindings: CloudflareBindings; Variables: { session: Session | null; data: Data; requestId: string } };
