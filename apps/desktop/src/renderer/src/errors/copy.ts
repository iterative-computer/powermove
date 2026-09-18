/** Copying diagnostics is offered from both the panel notice and the toast.
    Each surface owns its own label state; the outcome wording is shared. */
export async function copyErrorDetails(details: string): Promise<string> {
  try {
    await navigator.clipboard.writeText(details);
    return 'Copied';
  } catch {
    return 'Select the details to copy';
  }
}
