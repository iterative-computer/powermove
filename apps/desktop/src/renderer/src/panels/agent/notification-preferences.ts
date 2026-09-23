import { bridge } from '../../kernel/bridge';

/** The approved Powermove completion sound, bundled with the desktop app. */
export function notifyAgentFinished(): void {
  void bridge()?.agentNotification?.({ sound: 'Little Victory (Deep)' }).catch(() => {});
}
