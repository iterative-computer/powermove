import type { IpcMain, IpcMainInvokeEvent, WebContents } from 'electron';
import { StudioSurfaceReceiver } from './surface';
import { StudioHostManager } from './manager';
import { STUDIO_NATIVE_IPC, STUDIO_MAX_BINARY, isStudioNativeCommand, type StudioNativeCommand } from '../../shared/studio-native';

/** No paths, executable names, or OS handles are accepted from the page. */
export function registerStudioNativeIpc(ipc: Pick<IpcMain, 'handle' | 'removeHandler'>, executable: string | null,
  trusted: (event: IpcMainInvokeEvent) => boolean): () => void {
  const sessions = new Map<number, Map<string, StudioHostManager>>();
  const watched = new Set<number>();
  const imports = new Map<number,StudioHostManager>();
  const surfaces=new Map<StudioHostManager,StudioSurfaceReceiver>();
  const releaseManager=(manager:StudioHostManager)=>{manager.dispose();surfaces.get(manager)?.close();surfaces.delete(manager);};
  const check = (event: IpcMainInvokeEvent, key: unknown): string => {
    if (!trusted(event)) throw new Error('Unauthorized Studio IPC sender');
    if (typeof key !== 'string' || !/^[a-zA-Z0-9:_-]{1,128}$/.test(key)) throw new Error('Invalid Studio session');
    return key;
  };
  const releaseAll = (id: number) => { for (const manager of sessions.get(id)?.values() ?? []) releaseManager(manager); sessions.delete(id); imports.get(id)?.dispose();imports.delete(id); };
  const watch = (contents: WebContents) => {
    if (watched.has(contents.id)) return;
    watched.add(contents.id);
    contents.on('render-process-gone', () => releaseAll(contents.id));
    contents.on('did-start-navigation', (_event, _url, inPlace, mainFrame) => { if (mainFrame && !inPlace) releaseAll(contents.id); });
    contents.once('destroyed', () => { releaseAll(contents.id); watched.delete(contents.id); });
  };
  ipc.handle(STUDIO_NATIVE_IPC.request, async (event, key: unknown, command: StudioNativeCommand, binary?: Uint8Array) => {
    const id = check(event, key);
    if (!isStudioNativeCommand(command)) throw new Error('Invalid Studio command');
    if (binary !== undefined && (!(binary instanceof Uint8Array) || binary.byteLength > STUDIO_MAX_BINARY)) throw new Error('Invalid Studio binary payload');
    if(command?.op==='importFbx') {
      if(!binary?.byteLength)throw new Error('FBX data is empty');
      if(imports.has(event.sender.id))throw new Error('An FBX import is already running');
      if(imports.size >= 1)throw new Error('An FBX import is already running in another window');
      const worker=new StudioHostManager({executable,timeoutMs:120_000});
      imports.set(event.sender.id,worker);watch(event.sender);
      try {return await worker.request({op:'importFbx'},binary);}
      finally {worker.dispose();if(imports.get(event.sender.id)===worker)imports.delete(event.sender.id);}
    }
    let owner = sessions.get(event.sender.id);
    if (!owner) { owner = new Map(); sessions.set(event.sender.id, owner); watch(event.sender); }
    let manager = owner.get(id);
    if (!manager) {
      // Bound the native memory budget; the backend releases idle sessions.
      const active = [...sessions.values()].reduce((n, entries) => n + entries.size, 0);
      // One session for each pane of the supported 4 Views layout. Keep the
      // same process-wide cap across windows, rather than allowing four each.
      if (active >= 4) throw new Error('Too many active Studio sessions');
      const surface=StudioSurfaceReceiver.open(executable);
      manager = new StudioHostManager({ executable, surfaceService:surface?.service }); owner.set(id, manager);
      if(surface)surfaces.set(manager,surface);
    }
    const receiver=surfaces.get(manager);
    const reply=await manager.request(receiver?command:{...command,sharedTexture:false},binary);
    if(typeof reply.result?.surface==='number') {
      try {if(!receiver)throw new Error('Studio surface transport is unavailable');await receiver.present(event.sender,id,reply,manager.processId,()=>{if(manager!.status().running&&manager!.status().generation===reply.generation)void manager!.request({op:'releaseFrame',frame:reply.result.surface}).catch(()=>{});});}
      catch {return manager.request({...command,sharedTexture:false,goal:true,reset:false},binary);}
    }
    return reply;
  });
  ipc.handle(STUDIO_NATIVE_IPC.release, (event, key: unknown) => { const id = check(event, key); const owner = sessions.get(event.sender.id); const manager=owner?.get(id); if(manager)releaseManager(manager); owner?.delete(id); });
  ipc.handle(STUDIO_NATIVE_IPC.status, event => {
    if (!trusted(event)) throw new Error('Unauthorized Studio IPC sender');
    const manager = sessions.get(event.sender.id)?.values().next().value;
    return manager?.status() ?? { available: !!executable, running: false, generation: 0, failure: executable ? null : 'Native Studio is not installed' };
  });
  return () => { for (const id of new Set([...sessions.keys(),...imports.keys()])) releaseAll(id); for (const channel of Object.values(STUDIO_NATIVE_IPC)) ipc.removeHandler(channel); };
}
