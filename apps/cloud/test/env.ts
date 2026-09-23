import type { Data } from '../src/db/client';
import { FakeR2 } from './r2';
export function makeEnv(_data:Data):CloudflareBindings { return { DATABASE_URL:'postgresql://localhost/test', BETTER_AUTH_SECRET:'test-secret-at-least-32-characters-long', APP_ORIGIN:'https://cloud.trypowermove.com', HANDLE_MAIL_DOMAIN:'users.trypowermove.com', OBJECTS:new FakeR2(), TARS:new FakeR2(), ICONS:new FakeR2(), RL_UPLOAD:{limit:async()=>({success:true})} as RateLimit, RL_AUTH:{limit:async()=>({success:true})} as RateLimit }; }
