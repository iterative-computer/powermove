import { user } from '../src/db/auth-schema';
import { createAuth } from '../src/auth';
import type { Data } from '../src/db/client';
export async function seedSession(data:Data, env:CloudflareBindings, email='jude@example.com') {
  // Better Auth mints 32-char alphanumeric ids, not UUIDs; seed the same shape so
  // wire schemas that wrongly demand a UUID for user ids fail here, not in the app.
  const id=Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) => 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'[b % 62]).join('');
  await data.authDb().insert(user).values({id,name:'Jude',email,emailVerified:true});
  const auth=createAuth(data,env);
  const session=await (await auth.$context).internalAdapter.createSession(id);
  if(!session) throw new Error('session not created');
  return {id,token:session.token,headers:{Authorization:`Bearer ${session.token}`}};
}
