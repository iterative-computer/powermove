import { bridge } from '../../kernel/capture-bridge';
import { registerStudioBackend } from '../scene3d/studio-backend';
import { WebGpuStudioBackend } from '../studio/webgpu-backend';
import { NativeStudioBackend } from './backend';
import { RecoveringStudioBackend } from './recovery';

/** Called after capturing preload capability and before the editor's first scene. */
export function installNativeStudio(): void {
  const native = bridge()?.studioNative;
  if (!native) return;
  registerStudioBackend(context => new RecoveringStudioBackend(new NativeStudioBackend(native), () => new WebGpuStudioBackend(), context));
}
