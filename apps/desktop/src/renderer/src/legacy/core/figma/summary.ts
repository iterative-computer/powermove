import type { Issue } from '../../../../../shared/figma/convert';

const MISSING_FONT = /^Missing font:\s*/;
const list = (names: string[], limit = 3) =>
  names.length <= limit ? names.join(', ') : `${names.slice(0, limit).join(', ')} and ${names.length - limit} more`;

/** Plain-language result of a Figma import, for the import toast. */
export function figmaImportSummary(input: {
  update: boolean;
  layers: number;
  issues: Issue[];
  missing: string[];
  warnings: Issue[];
  clipboard: boolean;
}): { message: string; fonts: string[]; warned: string[] } {
  const { update, layers, issues, missing, warnings, clipboard } = input;
  const parts = [`${update ? 'Updated' : 'Imported'} ${layers} ${layers === 1 ? 'layer' : 'layers'}`];
  if (issues.length)
    parts[0] += ` · ${issues.length} flattened (${issues
      .slice(0, 3)
      .map((issue) => `${issue.name}: ${issue.reason}`)
      .join('; ')})${clipboard ? '. Send to Powermove gives better results.' : ''}`;
  if (missing.length) parts[0] += ` · Kept missing layers: ${missing.join(', ')}`;
  const fonts = [...new Set(warnings.filter((w) => MISSING_FONT.test(w.reason)).map((w) => w.reason.replace(MISSING_FONT, '').trim()))].filter(Boolean);
  if (fonts.length)
    parts.push(
      `${fonts.length === 1 ? '1 font' : `${fonts.length} fonts`} missing: ${list(fonts)}. Text stays editable with a substitute font.`,
    );
  const other = [...new Set(warnings.filter((w) => !MISSING_FONT.test(w.reason)).map((w) => w.reason))];
  if (other.length) parts.push(`${other.length === 1 ? 'Note' : 'Notes'}: ${list(other, 2)}.`);
  const message = parts.reduce((out, part) => (out ? out + (/[.!?]$/.test(out) ? ' ' : '. ') + part : part), '');
  return { message, fonts, warned: [...new Set(warnings.map((w) => w.id))] };
}
