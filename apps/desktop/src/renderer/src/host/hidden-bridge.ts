import type { PowermoveBridge } from '../../../shared/ipc';
import { ENGINE_FONTS_IPC } from '../../../shared/engine-fonts-ipc';
import { ENGINE_IPC } from '../../../shared/engine-ipc';
import { encodeFrame, decodeFrame } from '../../../shared/wire';
import type { ReconnectingLink } from '../../../shared/reconnect';
import { hiddenMediaStore } from './hidden-media-store';

export function nativeHiddenBridge(link: ReconnectingLink): Pick<PowermoveBridge, 'nativeEngine' | 'engineFonts' | 'mediaStore'> {
  const project = () => String((window as any).PM?.proj?.id ?? 'powermove-home');
  const payload = (channel: string, args: unknown[]) => encodeFrame({ project: project(), channel, args });
  const asyncCall = async <T>(channel: string, ...args: unknown[]): Promise<T> => {
    const response = await fetch('/__powermove/native-fonts', { method: 'POST', body: payload(channel, args) as BodyInit });
    if (!response.ok) throw new Error(`font-service-unavailable: HTTP ${response.status}`);
    return decodeFrame(new Uint8Array(await response.arrayBuffer())) as T;
  };
  const syncCall = <T>(channel: string, ...args: unknown[]): T => {
    const xhr = new XMLHttpRequest(); xhr.open('POST', '/__powermove/native-fonts', false);
    xhr.overrideMimeType('text/plain; charset=x-user-defined'); xhr.send(new Uint8Array(payload(channel, args)).buffer);
    if (xhr.status !== 200) throw new Error(`font-service-unavailable: HTTP ${xhr.status}`);
    return decodeFrame(Uint8Array.from(xhr.responseText, c => c.charCodeAt(0) & 255)) as T;
  };
  const fonts: NonNullable<PowermoveBridge['engineFonts']> = {
    answerSync: q => syncCall(ENGINE_FONTS_IPC.answerSync, q), wholeSync: (b, i) => syncCall(ENGINE_FONTS_IPC.wholeSync, b, i),
    answer: q => asyncCall(ENGINE_FONTS_IPC.answer, q), subset: (b, i) => asyncCall(ENGINE_FONTS_IPC.subset, b, i),
    manifest: b => asyncCall(ENGINE_FONTS_IPC.manifest, b), familyFaces: f => asyncCall(ENGINE_FONTS_IPC.familyFaces, f),
    addProjectFont: (f, d, desc, i) => asyncCall(ENGINE_FONTS_IPC.addProjectFont, f, d, desc, i),
    setFontSource: (f, d) => asyncCall(ENGINE_FONTS_IPC.setFontSource, f, d), clearProjectFonts: () => asyncCall(ENGINE_FONTS_IPC.clearProjectFonts),
    generation: () => asyncCall(ENGINE_FONTS_IPC.generation), seeds: () => asyncCall(ENGINE_FONTS_IPC.seeds),
    reportUiFonts: json => asyncCall(ENGINE_FONTS_IPC.uiFonts, json)
  };
  return { engineFonts: fonts, mediaStore: hiddenMediaStore(link), nativeEngine: {
    enabledSync: () => true,
    status: () => link.invoke(ENGINE_IPC.getStatus), hostTime: () => link.invoke(ENGINE_IPC.hostTime),
    pushDefinitions: update => link.send(ENGINE_IPC.definitions, update)
  } as NonNullable<PowermoveBridge['nativeEngine']> };
}
