/* Ported from host/electron-shim.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
  'use strict';

  if (!(window as any).powermove) return;

  const bridge = (window as any).powermove;
  const encoder = new window.TextEncoder();

  function bytesFromBase64(value: any) {
    const normalized = String(value || '').replace(/\s+/g, '').replace(/-/g, '+').replace(/_/g, '/');
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=');
    return Uint8Array.from(window.atob(padded), (character: any) => character.charCodeAt(0));
  }

  function decodeBinary(value: any) {
    const source = String(value || '');
    const comma = source.indexOf(',');
    if (source.startsWith('data:') && comma >= 0) {
      const metadata = source.slice(5, comma);
      const payload = source.slice(comma + 1);
      return /(?:^|;)base64(?:;|$)/i.test(metadata)
        ? bytesFromBase64(payload)
        : encoder.encode(decodeURIComponent(payload));
    }
    return bytesFromBase64(source);
  }

  function bytesToBase64(value: any) {
    const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
    }
    return window.btoa(binary);
  }

  function textToBase64(value: any) {
    return bytesToBase64(encoder.encode(String(value ?? '')));
  }

  function cloneValue(value: any) {
    return window.structuredClone(value);
  }

  function binaryList(values: any) {
    const result = [];
    for (const value of Array.isArray(values) ? values : []) {
      try { result.push(decodeBinary(value)); } catch { /* Main validates the remaining images. */ }
    }
    return result;
  }

  function attachmentBytes(attachment: any) {
    if (typeof attachment?.dataUrl === 'string') return decodeBinary(attachment.dataUrl);
    if (typeof attachment?.dataBase64 === 'string') return decodeBinary(attachment.dataBase64);
    if (typeof attachment?.data === 'string') return decodeBinary(attachment.data);
    return encoder.encode(String(attachment?.content ?? ''));
  }

  function mapBody(body: any = {}) {
    const mode = body.mode === 'autonomous' ? 'autonomous' : 'editor';
    const access = mode === 'editor'
      ? 'editor'
      : (body.access === 'computer' ? 'computer' : 'project');
    const requestedEffort = String(body.reasoningEffort || '');
    const reasoningEffort = ['low', 'medium', 'high'].includes(requestedEffort)
      ? requestedEffort
      : (['xhigh', 'max'].includes(requestedEffort) ? 'high' : null);
    const attachments = [];
    for (const attachment of Array.isArray(body.attachments) ? body.attachments : []) {
      try {
        attachments.push({
          name: String(attachment?.name || 'attachment.txt'),
          data: attachmentBytes(attachment)
        });
      } catch { /* Main validates the remaining attachments. */ }
    }

    return {
      id: String(body.id || ''),
      mode,
      prompt: String(body.prompt || ''),
      schema: body.schema && typeof body.schema === 'object' && !Array.isArray(body.schema)
        ? body.schema : null,
      images: binaryList(body.images),
      model: typeof body.model === 'string' && body.model ? body.model : null,
      reasoningEffort,
      access,
      projectId: String(body.projectId || (mode === 'editor' ? 'editor' : '')),
      projectName: String(body.projectName || ''),
      projectJSON: typeof body.projectJSON === 'string' && body.projectJSON ? body.projectJSON : null,
      attachments,
      consentToken: typeof body.consentToken === 'string' && body.consentToken ? body.consentToken : null
    };
  }

  function errorText(error: any, fallback: any) {
    return String(error?.message || error || fallback);
  }

  function savedName(path: any, fallback: any) {
    return String(path || '').split(/[\\/]/).pop() || fallback;
  }

  function logMessage(body: any) {
    const text = String(body ?? '');
    const tag = /^\[(error|warn|uncaught|rejection)\]\s*/.exec(text)?.[1];
    const level = tag === 'warn' ? 'warn'
      : tag === 'uncaught' ? 'uncaught'
        : (tag === 'error' || tag === 'rejection' ? 'error' : 'info');
    bridge.log(level, text.slice(0, 8000));
  }

  const noOp = { postMessage() {} };

  (window as any).webkit = {
    messageHandlers: {
      saveFile: {
        postMessage(body: any = {}) {
          void (async () => {
            try {
              const name = String(body.name || 'powermove.bin');
              const result = await bridge.saveFile({ name, data: decodeBinary(body.data) });
              if (result.ok) PM.toast(`Saved ${savedName(result.path, name)}`);
              else if (!result.cancelled) PM.toast(result.error || 'Save failed');
            } catch (error) {
              PM.toast(errorText(error, 'Save failed'));
            }
          })();
        }
      },
      windowDrag: noOp,
      windowZoom: noOp,
      pmLog: {
        postMessage: logMessage
      },
      pmTheme: {
        postMessage(body: any) {
          bridge.setTheme(['light', 'dark', 'system'].includes(body) ? body : 'system');
        }
      },
      pmPanelTitle: noOp,
      pmCodex: {
        postMessage(body: any = {}) {
          const id = String(body.id || '');
          void (async () => {
            let request = mapBody(body);
            if (request.access === 'computer' && !request.consentToken) {
              const consent = await bridge.codex.requestComputerConsent({
                projectName: request.projectName || 'Untitled',
                summary: request.prompt.slice(0, 500)
              });
              if (!consent.granted) {
                PM.CodexBridge.resolve(id, {
                  ok: false,
                  dataBase64: textToBase64('Computer access was not allowed')
                });
                return;
              }
              request = { ...request, consentToken: consent.token };
            }
            const result = await bridge.codex.run(request, (text: any) => {
              PM.CodexBridge.progress(id, {
                dataBase64: textToBase64(text)
              });
            });
            PM.CodexBridge.resolve(id, {
              ok: result.ok,
              dataBase64: textToBase64(result.ok ? result.text : result.error),
              extensions: result.ok ? result.extensions : undefined
            });
          })().catch(error => {
            PM.CodexBridge.resolve(id, {
              ok: false,
              dataBase64: textToBase64(errorText(error, 'The coding agent failed'))
            });
          });
        }
      },
      pmCodexCancel: {
        postMessage(body: any = {}) {
          void bridge.codex.cancel(String(body.id || '')).catch(() => {});
        }
      },
      pmAgentArtifact: {
        postMessage(body: any = {}) {
          const id = String(body.id || '');
          void bridge.artifacts.read({
            projectId: String(body.projectId || ''),
            path: String(body.path || '')
          }).then((file: any) => {
            PM.AgentArtifacts.resolve(id, {
              ok: true,
              name: file.name,
              mime: file.mime,
              dataBase64: bytesToBase64(file.data)
            });
          }).catch((error: any) => {
            PM.AgentArtifacts.resolve(id, {
              ok: false,
              name: '',
              mime: '',
              dataBase64: '',
              message: errorText(error, 'The artifact could not be loaded')
            });
          });
        }
      },
      pmAgentReveal: {
        postMessage(body: any = {}) {
          void bridge.artifacts.reveal({
            projectId: String(body.projectId || ''),
            path: String(body.path || '')
          }).catch((error: any) => PM.toast(errorText(error, 'The artifact could not be revealed')));
        }
      },
      pmCaptureWindow: {
        postMessage(body: any = {}) {
          const id = String(body.id || '');
          void bridge.captureWindow().then((data: any) => {
            PM.WindowCapture.resolve(id, {
              ok: data !== null,
              dataBase64: data === null ? '' : bytesToBase64(data)
            });
          }).catch(() => {
            PM.WindowCapture.resolve(id, { ok: false, dataBase64: '' });
          });
        }
      }
    }
  };

  let snapshot = {};
  try {
    snapshot = bridge.store.snapshotSync() || {};
  } catch (error) {
    bridge.log('error', `store snapshot failed: ${errorText(error, 'unknown error')}`);
  }
  const cached = new Map(Object.entries(snapshot).map(([key, value]) => [`pm.${key}`, value]));
  const serviceKey = (key: any) => String(key).startsWith('pm.') ? String(key).slice(3) : String(key);
  const legacyKey = (key: any) => `pm.${serviceKey(key)}`;

  PM.store = {
    get(key: any, fallback: any) {
      const stored = legacyKey(key);
      if (!cached.has(stored)) return fallback;
      try { return cloneValue(cached.get(stored)); } catch { return fallback; }
    },
    set(key: any, value: any) {
      const service = serviceKey(key);
      try {
        const copy = cloneValue(value);
        bridge.store.set(service, copy);
        cached.set(legacyKey(service), copy);
      } catch (error) {
        window.console.warn('store', error);
      }
    },
    del(key: any) {
      const service = serviceKey(key);
      cached.delete(legacyKey(service));
      bridge.store.delete(service);
    }
  };

  bridge.store.onError((event: any) => {
    if (String(event.error).includes('unknown store key')) {
      cached.delete(legacyKey(event.key));
    }
    PM.toast(`Could not save ${event.key}: ${event.error}`);
  });

  async function publishSystemFonts() {
    try {
      if (typeof (window as any).queryLocalFonts === 'function') {
        const fonts = await (window as any).queryLocalFonts();
        const families = [...new Set(fonts.map((font: any) => font.family).filter(Boolean))];
        PM.Fonts?.setSystemFamilies(families);
        return true;
      }
    } catch { /* PM.Fonts retains its bundled and web fallback families. */ }
    return false;
  }

  async function finishBoot() {
    window.document.documentElement.classList.add('native-app');
    if (!await publishSystemFonts() && typeof (window as any).queryLocalFonts === 'function') {
      window.addEventListener('pointerdown', () => { void publishSystemFonts(); }, {
        capture: true,
        once: true
      });
    }

    bridge.onMenuCommand((command: any) => PM.cmd && PM.cmd(command));
    window.setTimeout(() => {
      try {
        const ok = !!(PM.GL && PM.GL.gl);
        const layers = PM.proj ? PM.proj.layers.length : -1;
        const milliseconds = PM.perf ? (PM.perf.ms || 0).toFixed(1) : '?';
        bridge.log('info', `boot gl=${ok} layers=${layers} ms=${milliseconds}`);
      } catch { /* Boot diagnostics must never interrupt the renderer. */ }
    }, 1200);
  }

  if (window.document.readyState === 'loading') {
    window.document.addEventListener('DOMContentLoaded', finishBoot, { once: true });
  } else {
    void finishBoot();
  }
}
