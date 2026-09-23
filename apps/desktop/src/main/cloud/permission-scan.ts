import { scanCapabilities } from '@powermove/registry/scan';
import type { ExtensionPermission } from '../../shared/extensions';
import type { PermissionFinding } from '../../shared/publish';

const SOURCE = /\.(?:[cm]?[jt]sx?|svelte)$/i;
const TRUSTED = /\bapi\s*\.\s*(?:render|host|services|inspector|anim|model|history|edit|groups|uiState|dnd|workspace|ui\s*\.\s*(?:controls|modal|menu|drag|gesture|mount)|media\s*\.\s*(?:importFiles|assets|audio|fonts))\b/g;
const NETWORK = /\b(?:fetch\s*\(|new\s+WebSocket\s*\(|XMLHttpRequest\b|new\s+EventSource\s*\(|navigator\s*\.\s*sendBeacon\b)/;
const CLIPBOARD = /\bnavigator\s*\.\s*clipboard\b/;

/** Static hints for direct calls. Destructured aliases and dynamic property access are out of scope. */
export function permissionFindings(files: { path: string; text: string }[], permissions: readonly ExtensionPermission[] = []): PermissionFinding[] {
  const declared = new Set(permissions);
  const sources = files.filter((file) => SOURCE.test(file.path));
  const findings: PermissionFinding[] = [];
  for (const finding of scanCapabilities(sources)) {
    if (declared.has(finding.capability)) continue;
    const line = sources.find((file) => file.path === finding.path)?.text.split(/\r?\n/)[finding.line - 1] ?? '';
    const token = finding.capability === 'network' ? (NETWORK.exec(line)?.[0].trim() ?? 'network access') : (CLIPBOARD.exec(line)?.[0] ?? 'navigator.clipboard');
    const name = token.replace(/\s+/g, ' ').replace(/\s*\($/, '()');
    findings.push({ path: finding.path, line: finding.line, needs: finding.capability,
      text: `Uses ${name} at ${finding.path}:${finding.line} but doesn't declare the ${finding.capability} permission. Add \`permissions: ["${finding.capability}"]\` to manifest.json.` });
  }
  for (const file of sources) for (const [index, line] of file.text.split(/\r?\n/).entries()) {
    TRUSTED.lastIndex = 0;
    for (const match of line.matchAll(TRUSTED)) {
      if (declared.has('full-access')) break;
      const name = match[0].replace(/\s+/g, '');
      findings.push({ path: file.path, line: index + 1, needs: 'full-access',
        text: `Uses ${name} (full access) at ${file.path}:${index + 1} but doesn't declare the full-access permission. Add \`permissions: ["full-access"]\` to manifest.json.` });
    }
  }
  return findings.sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.needs.localeCompare(b.needs));
}
