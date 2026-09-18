export interface ErrorPresentation {
  title: string;
  message: string;
  details: string;
}

/** What a failed turn actually was, and so how much apparatus it earns.
    `error` is the editor or the transport failing, and keeps the diagnostics.
    `alert` is the agent reporting that it could not do the thing, with the
    project intact — worth marking, but there is nothing to decipher. `plain`
    is the agent simply answering: nothing broke, nothing was lost, and a red
    card would overstate it. */
export type NoticeKind = 'plain' | 'alert' | 'error';

const NOTICE_KINDS: readonly string[] = ['plain', 'alert', 'error'];

/** An error carries its own kind when the code that threw it knew; anything
    unrecognised stays an error, so a real fault is never quietly softened. */
export function noticeKind(value: unknown): NoticeKind {
  const stated = (value as { notice?: unknown } | null)?.notice;
  return typeof stated === 'string' && NOTICE_KINDS.includes(stated) ? stated as NoticeKind : 'error';
}

/** Throw one of these where the outcome is known: `stated('…', 'plain')`. */
export function stated(message: string, kind: NoticeKind): Error {
  return Object.assign(new Error(message), { notice: kind });
}

/** Keep diagnostics available without asking people to decipher transport logs. */
export function presentError(value: unknown): ErrorPresentation {
  const details = (value instanceof Error ? value.message : String(value ?? '')).trim();
  const result = (title: string, message: string): ErrorPresentation => ({ title, message, details });
  if (/active writer|saved agent session couldn[’']t reopen/i.test(details)) {
    return result('Agent session couldn’t reopen', 'Try sending your request again. If it repeats, restart Powermove to reconnect the agent.');
  }
  if (/already (?:active|running).*conversation|run .*already active/i.test(details)) {
    return result('Conversation is busy', 'Wait for the current run to finish, or stop it before sending your request again.');
  }
  if (/no rollout found|(?:thread|session).{0,80}(?:not found|could not be resumed)/i.test(details)) {
    return result('Couldn’t reopen the conversation', 'Send your request again. If this continues, start a new conversation.');
  }
  if (/rate.?limit|\b429\b|overloaded|usage limit/i.test(details)) {
    return result('Agent is temporarily unavailable', 'Wait a moment, then send your request again. Check your account if you’ve reached a usage limit.');
  }
  if (/not signed in|not logged in|login required|unauthorized|\b401\b|authentication|sign.{0,8}again/i.test(details)) {
    return result('Sign-in needs attention', 'Reconnect your agent account in Settings, then send your request again.');
  }
  if (/too long|timed? out|timeout/i.test(details)) {
    return result('The request timed out', 'Check the current result before trying again; the agent may have completed part of the work.');
  }
  if (/ENOSPC|disk.{0,10}full|no space left/i.test(details)) {
    return result('Your disk is full', 'Free up some storage, then try the operation again.');
  }
  if (/EACCES|EPERM|permission denied/i.test(details)) {
    return result('File access was denied', 'Choose a location you can access, or check the app’s file permissions in System Settings.');
  }
  if (/network|ECONNRESET|ENOTFOUND|fetch failed|connection.{0,30}(?:interrupted|closed|refused)|stream disconnected/i.test(details)) {
    return result('Connection interrupted', 'Check your internet connection, then try again.');
  }
  if (/invalid (?:result|section)|returned.{0,30}invalid|response format.*rejected|without a result/i.test(details)) {
    return result('The agent couldn’t finish the result', 'Review any changes already made, then send your request again.');
  }
  const readable = details.replace(/^(?:The agent failed:\s*|Error:\s*|Error invoking remote method '[^']+':\s*)+/i, '');
  const technical = /(?:\b(?:TypeError|ReferenceError|SyntaxError|IpcValidationError)\b|\bremote method\b|\bat \S+ \(|thread\/|code -\d+|\b[A-Z_]{4,}:)/.test(readable);
  return result('Something needs attention', !technical && readable.length <= 400 && readable
    ? readable : 'This operation couldn’t finish. Try again; if it continues, copy the details to help diagnose the problem.');
}
