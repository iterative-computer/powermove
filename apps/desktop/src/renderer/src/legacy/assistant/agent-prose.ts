/** Keep Markdown line structure in the final agent summary. */
export function normalizeAgentSummary(value: unknown): string {
  return String(value || '').replace(/\r\n?/g, '\n')
    .replace(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/g, ' ').trim()
    .slice(0, 30_000);
}
