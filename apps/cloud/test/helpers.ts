import { user } from '../src/db/auth-schema';
import { createAuth } from '../src/auth';
import type { Data } from '../src/db/client';
export async function seedSession(data:Data, env:CloudflareBindings, email='jude@example.com') {
  const id=crypto.randomUUID();
  await data.authDb().insert(user).values({id,name:'Jude',email,emailVerified:true});
  const auth=createAuth(data,env);
  const session=await (await auth.$context).internalAdapter.createSession(id);
  if(!session) throw new Error('session not created');
  return {id,token:session.token,headers:{Authorization:`Bearer ${session.token}`}};
}
