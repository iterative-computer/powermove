/** `engine:proxies`: the renderer's proxy controller ⇄ EngineProxyService. Every field is validated here; owners are the sender's webContents id. */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { ENGINE_IPC } from '../../shared/engine-ipc';
import { validGeneratedMediaLocation, type ProxyAssetDescriptor } from '../../shared/proxies';
import { validProxyIntent, type EngineProxyService } from './proxies';
import { isId, isRecord } from './protocol';

const MAX_ASSETS = 100_000;
const HASH = /^[0-9a-f]{64}$/;

/** The descriptors the service may use. Renderer paths are dropped: originals come from the media store. */
export function proxyDescriptors(value: unknown): Record<string, ProxyAssetDescriptor> {
  if (!isRecord(value) || Object.keys(value).length > MAX_ASSETS) throw new Error('Invalid project proxy settings.');
  const descriptors: Record<string, ProxyAssetDescriptor> = {};
  for (const [asset, meta] of Object.entries(value)) {
    if (!isId(asset) || !isRecord(meta)) throw new Error('Invalid proxy asset.');
    descriptors[asset] = {
      ...(typeof meta['contentHash'] === 'string' && HASH.test(meta['contentHash']) ? { contentHash: meta['contentHash'] } : {}),
      ...(typeof meta['kind'] === 'string' ? { kind: meta['kind'].slice(0, 32) } : {}),
      ...(typeof meta['name'] === 'string' ? { name: meta['name'].slice(0, 1024) } : {}),
      ...(meta['imageSequence'] === true ? { imageSequence: true } : {}),
      ...(typeof meta['originalSequenceHash'] === 'string' && HASH.test(meta['originalSequenceHash']) ? { originalSequenceHash: meta['originalSequenceHash'] } : {}),
      ...(meta['alwaysPlayOriginal'] === true ? { alwaysPlayOriginal: true } : {})
    };
  }
  return descriptors;
}

export function installProxyIpc(options: {
  ipc: IpcMain;
  trusted(event: IpcMainInvokeEvent): boolean;
  service: EngineProxyService;
  chooseFolder(event: IpcMainInvokeEvent): Promise<string | null>;
}): () => void {
  options.ipc.handle(ENGINE_IPC.proxies, async (event, request: unknown) => {
    if (!options.trusted(event)) throw new Error('Unauthorized IPC sender');
    if (!isRecord(request) || typeof request['action'] !== 'string') throw new Error('Invalid proxy request.');
    const action = request['action'], owner = event.sender.id;
    if (action === 'chooseFolder') return options.chooseFolder(event);
    if (action === 'location') return options.service.location();
    if (action === 'setLocation') {
      if (!validGeneratedMediaLocation(request['location'])) throw new Error('Choose a generated media location.');
      return options.service.setLocation(request['location']);
    }
    const project = request['project'];
    if (!isId(project)) throw new Error('A proxy request names an open project.');
    if (action === 'snapshot') return options.service.snapshot(owner, project);
    if (action === 'setIntent') {
      if (!validProxyIntent(request['intent'])) throw new Error('Invalid project proxy settings.');
      const projectPath = request['projectPath'];
      const saved = typeof projectPath === 'string' && projectPath.startsWith('/') && projectPath.length <= 4096 && !projectPath.includes('\0');
      return options.service.setIntent(owner, project, request['intent'], proxyDescriptors(request['assets']), saved ? projectPath : undefined);
    }
    const assets = request['assets'];
    if (!Array.isArray(assets) || assets.length > MAX_ASSETS || !assets.every(isId)) throw new Error('A proxy action names assets.');
    if (action === 'create') return options.service.create(owner, project, assets);
    if (action === 'rebuild') return options.service.rebuild(owner, project, assets);
    if (action === 'remove') return options.service.remove(owner, project, assets);
    if (action === 'slow') return options.service.slow(owner, project, assets);
    if (action === 'prioritize') return options.service.prioritize(owner, project, assets);
    throw new Error('Unknown proxy action.');
  });
  return () => { options.ipc.removeHandler(ENGINE_IPC.proxies); };
}
