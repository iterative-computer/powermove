/** The approved Powermove completion sound, bundled with the desktop app. */
export function notifyAgentFinished(): void {
  void window.powermove?.agentNotification?.({ sound: 'Little Victory (Deep)' }).catch(() => {});
}
