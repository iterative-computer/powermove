/** Shared verification rules, with extension completion only when relevant. */
export const AGENT_TESTING_INSTRUCTIONS = `BACKGROUND TESTING
Never open or show a new window for testing. Use window-free unit/type checks, headless browsers or a hidden Electron renderer with isolated temporary test data. Keep the user's process, window and editing session open; hot reload extensions, never quit/relaunch or reload the whole window. Native OS behavior requires explicit user coordination.
Powermove's source checkout, package scripts, and Electron test harness are not available. Verify through live panel state/interaction/capture, workspace errors, render_frames and the compile report. Never test against the live profile or change the user's document merely for a test. Report unavailable checks; never substitute a visible test window.

EXTENSION COMPLETION
Read shipped API types; never invent host services or validate with mocks that repeat assumptions. Native raster surfaces use cv; when wrapping api.services capture the previous service and preserve surface geometry.
Files are staged until returned in extensions. After promotion Powermove continues with a load report: inspect actual controls, screenshots, rendered frames and errors. Repair and verify failures yourself; do not claim staged files were live-tested or defer repair to a Fix it button. Report concrete blockers.

TOOL RELIABILITY
Confirm working directory and uncertain paths before commands. Separate discovery from edits; expected absence must exit cleanly. On failure read the exact result, correct the smallest operation and verify the retry. Inspect state before retrying possibly completed edits. Never repeat an identical failed call or claim a failed check passed.`;

/** Inspection/planning cannot author extensions. */
export const AGENT_EDITOR_TESTING_INSTRUCTIONS = AGENT_TESTING_INSTRUCTIONS.replace(/\n\nEXTENSION COMPLETION[\s\S]*?(?=\n\nTOOL RELIABILITY)/, '');

export function agentTestingInstructions(extensions: boolean): string {
  return extensions ? AGENT_TESTING_INSTRUCTIONS : AGENT_EDITOR_TESTING_INSTRUCTIONS;
}
