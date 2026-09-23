import {test,expect} from 'bun:test';
import {createApp} from '../src/app';
import {withData} from './db';
import {makeEnv} from './env';
test('error shapes, client floor and health',async()=>withData(async data=>{
 const env=makeEnv(data),app=createApp({data:()=>data});
 let r=await app.request('/v1/me',{},env);expect(r.status).toBe(401);expect(await r.json() as any).toEqual({error:'unauthorized'});
 r=await app.request('/v1/auth/email/send',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({email:'bad'})},env);expect(r.status).toBe(400);expect((await r.json() as any).error).toBe('bad_request');
 r=await app.request('/health',{headers:{'X-Powermove-Client':'desktop/0.0.0'}},env);expect(r.status).toBe(200);
 r=await app.request('/health',{headers:{'X-Powermove-Client':'desktop/broken'}},env);expect(r.status).toBe(426);expect(await r.json() as any).toEqual({error:'client_too_old',minimum:'0.0.0'});
 const failing=createApp({data:()=>{throw new Error('test error')}});const old=console.error;console.error=()=>{};try{r=await failing.request('/v1/me',{},env);}finally{console.error=old;}expect(r.status).toBe(500);expect(await r.json() as any).toEqual({error:'internal'});
}));
