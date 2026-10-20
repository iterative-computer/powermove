/** Shared verification rules, with extension completion only when relevant. */
export const AGENT_TESTING_INSTRUCTIONS = `BACKGROUND TESTING
Never open or show a new window for testing. Use unit/type checks, headless browsers or hidden Electron with isolated temporary test data. Keep the user's process/window/session; hot reload extensions, never quit/relaunch/reload the window. Coordinate native OS tests with the user.
The source checkout, package scripts, and Electron test harness are not available. Verify panel state/interaction/capture, workspace errors, render_frames, compile reports. Never test with the live profile/user document. Report unavailable checks.

EXTENSION COMPLETION
Read shipped API types; never invent host services or validate assumption-repeating mocks. Native raster: cv. Wrapping api.services: retain the previous service and surface geometry.
Files stay staged until returned in extensions. After promotion inspect load report, actual controls/screenshots/frames/errors. Repair/verify failures; never claim staged files were live-tested or defer to Fix it. Report blockers.

TOOL RELIABILITY
Confirm directory/uncertain paths. Discover before edits; expected absence exits cleanly. On failure read the exact result; correct/verify the smallest retry. Inspect state before retrying possibly completed edits. Never repeat identical failed calls or claim failed checks passed.`;

/** Inspection/planning cannot author extensions. */
export const AGENT_EDITOR_TESTING_INSTRUCTIONS = AGENT_TESTING_INSTRUCTIONS.replace(/\n\nEXTENSION COMPLETION[\s\S]*?(?=\n\nTOOL RELIABILITY)/, '');

export function agentTestingInstructions(extensions: boolean): string {
  return extensions ? AGENT_TESTING_INSTRUCTIONS : AGENT_EDITOR_TESTING_INSTRUCTIONS;
}
