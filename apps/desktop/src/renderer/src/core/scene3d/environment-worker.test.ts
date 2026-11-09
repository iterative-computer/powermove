import {afterEach,expect,it,vi} from 'vitest';
import {disposeEnvironmentWorkers,environmentPreparationIdle,prepareEnvironmentInWorker} from './environment-worker';
import type {EnvironmentJob,TerrainPart} from './environment-preparation';
class TestWorker {
 static instances:TestWorker[]=[];onmessage:any;onerror:any;inputs:EnvironmentJob[]=[];terminated=false;
 constructor(){TestWorker.instances.push(this);}
 postMessage(input:EnvironmentJob){this.inputs.push(input);}
 terminate(){this.terminated=true;}
 complete(){this.onmessage?.({data:{result:[]}});}
}
const input=(seed:number):EnvironmentJob=>({kind:'terrain',settings:{resolution:2,seed},palette:[],materials:[]});
afterEach(async()=>{disposeEnvironmentWorkers();await environmentPreparationIdle();vi.unstubAllGlobals();TestWorker.instances=[];});
it('supersedes queued edits and releases active and queued jobs when the project closes',async()=>{
 vi.stubGlobal('Worker',TestWorker);
 const active=prepareEnvironmentInWorker<TerrainPart[]>(input(1),{}),owner={};
 const old=prepareEnvironmentInWorker<TerrainPart[]>(input(2),owner).catch(error=>error.message);
 const latest=prepareEnvironmentInWorker<TerrainPart[]>(input(3),owner);
 expect(await old).toContain('superseded');expect(TestWorker.instances).toHaveLength(1);
 const worker=TestWorker.instances[0]!;worker.complete();await active;
 expect(worker.inputs.map(i=>i.settings.seed)).toEqual([1,3]);
 const closed=latest.catch(error=>error.message);disposeEnvironmentWorkers();
 expect(await closed).toContain('project closed');expect(worker.terminated).toBe(true);await environmentPreparationIdle();
});
