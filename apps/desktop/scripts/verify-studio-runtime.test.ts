import { beforeEach, describe, expect, it, vi } from 'vitest';
const fixture = vi.hoisted(() => ({ team:'example-team', badBinary:false, absoluteDependency:false, absoluteRpath:false, cpu:true, materialBytes:1088,triangleBytes:144 }));
vi.mock('node:fs/promises', () => ({
  access: vi.fn(async () => {}),
  readdir: vi.fn(async () => ['pm-studio-host','studio-surface.node','libOpenImageDenoise.2.dylib','libtbb.12.dylib',...(fixture.cpu?['libOpenImageDenoise_device_cpu.2.dylib']:[])])
}));
vi.mock('node:child_process', () => ({spawnSync:(command:string,args:string[])=>{
  if(command.endsWith('/pm-studio-host')){
    const json=Buffer.from(JSON.stringify({id:1,ok:true,result:{protocol:1,layout:{materialBytes:fixture.materialBytes,triangleBytes:fixture.triangleBytes,nodeBytes:64,instanceFloats:36}}})),header=Buffer.alloc(12);
    header.write('PMS1');header.writeUInt32LE(json.length,4);
    return {status:0,stdout:Buffer.concat([header,json]),stderr:Buffer.alloc(0)};
  }
  const file=args.at(-1)??'';
  const output=command.endsWith('codesign')
    ? args.includes('-dv')?`CodeDirectory flags=0x10000(runtime)\nTeamIdentifier=${fixture.badBinary&&file.endsWith('studio-surface.node')?'other-team':fixture.team}\n`:''
    : args[0]==='-L'?`binary:\n\t${fixture.absoluteDependency?'/opt/homebrew/lib/unsafe.dylib':'@rpath/libOpenImageDenoise.2.dylib'} (compatibility version 1.0.0)\n`
    : fixture.absoluteRpath?'cmd LC_RPATH\n cmdsize 32\n path /opt/homebrew/lib (offset 12)\n':'';
  return {status:0,stdout:output,stderr:''};
}}));
import { verifyStudioRuntime } from './verify-studio-runtime.mjs';
beforeEach(()=>Object.assign(fixture,{team:'example-team',badBinary:false,absoluteDependency:false,absoluteRpath:false,cpu:true,materialBytes:1088,triangleBytes:144}));

describe('packaged Studio signing gate',()=>{
  it('accepts a complete consistently signed runtime',async()=>{
    expect(await verifyStudioRuntime('/test/Powermove.app')).toContain('studio-surface.node');
  });
  it('rejects a surface addon signed by another team',async()=>{
    fixture.badBinary=true;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('signing policy');
  });
  it('rejects Homebrew dependencies and runtime search paths',async()=>{
    fixture.absoluteDependency=true;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('non-relocatable');
    fixture.absoluteDependency=false;fixture.absoluteRpath=true;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('absolute runtime search path');
  });
  it('keeps ad-hoc signing restricted to the local verification option',async()=>{
    fixture.team='not set';
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('hardened signature');
    await expect(verifyStudioRuntime('/test/Powermove.app',{adHoc:true})).resolves.toContain('pm-studio-host');
  });
  it('rejects a package that omits the OIDN CPU plugin',async()=>{
    fixture.cpu=false;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('CPU denoiser');
  });
  it('rejects a signed helper copied from an incompatible Cargo build',async()=>{
    fixture.materialBytes=128;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('does not match the renderer layout');
    fixture.materialBytes=1088;fixture.triangleBytes=96;
    await expect(verifyStudioRuntime('/test/Powermove.app')).rejects.toThrow('does not match the renderer layout');
  });
});
