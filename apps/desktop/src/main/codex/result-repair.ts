/** Only failures detected before publication may request a corrected result. */
export class AgentResultValidationError extends Error {}

export async function repairAgentResult<T>(
  complete: () => Promise<T>,
  repair: (prompt: string) => Promise<void>,
  onProgress?: (text: string) => void
): Promise<T> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await complete();
    } catch (error) {
      if (!(error instanceof AgentResultValidationError) || attempt >= 2) throw error;
      onProgress?.(`Correcting the agent result (${attempt + 1} of 2)…`);
      await repair(`Correct the completion report for the current run. Validation failed before publishing extensions or importing artifacts:
${error.message}

Continue in the same staging and artifact directories. Inspect the existing files and return a complete corrected structured result. Preserve the previous commands and artifact import requests unless they caused this validation error; those returned commands and imports have not been applied yet. Do not repeat live project edits, completed shell operations, uploads, or other side effects. Read get_project_state before any new project edit. Report only extension changes made in this stage, with their actual actions; omit unchanged extensions. Do not claim that an unsuccessful operation completed.`);
    }
  }
}
