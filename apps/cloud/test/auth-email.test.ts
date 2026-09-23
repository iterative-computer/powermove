import {test,expect} from 'bun:test';
import {createApp} from '../src/app';
import {withData} from './db';
import {makeEnv} from './env';
test('email OTP signs in and returns a usable bearer',async()=>withData(async data=>{
 const env=makeEnv(data),app=createApp({data:()=>data});let line='';const old=console.log;console.log=(...args)=>{line=args.join(' ')};
 try {const sent=await app.request('/v1/auth/email/send',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({email:'otp@example.com'})},env);expect(sent.status).toBe(200);}finally{console.log=old;}
 const otp=line.match(/: ([0-9]+)$/)?.[1];expect(otp).toBeDefined();
 const verified=await app.request('/v1/auth/email/verify',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({email:'otp@example.com',otp})},env);expect(verified.status).toBe(200);const session=await verified.json() as {token:string;expiresAt:string};expect(session.token).toBeTruthy();
 const me=await app.request('/v1/me',{headers:{Authorization:`Bearer ${session.token}`}},env);expect(me.status).toBe(200);
 const signout=await app.request('/v1/auth/sign-out',{method:'POST',headers:{Authorization:`Bearer ${session.token}`}},env);expect(signout.status).toBe(200);expect(await signout.json() as any).toEqual({ok:true});
 const after=await app.request('/v1/me',{headers:{Authorization:`Bearer ${session.token}`}},env);expect(after.status).toBe(401);
}));
