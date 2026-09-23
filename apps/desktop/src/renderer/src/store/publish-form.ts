/* The publish sheet's form, kept apart from the component so the rules are
   testable: what each field may hold, which problems block the button, and
   the `PublishForm` sent to main (which checks all of it again). */
import {
  PUBLISH_LICENCES,
  PUBLISH_LIMITS,
  findingKey,
  versionProblem,
  waiverProblem,
  type PublishCategory,
  type PublishForm,
  type PublishLicence,
  type PublishPlanDto,
  type PublishProgress,
  type PublishVisibility
} from '../../../shared/publish';

export { versionProblem, waiverProblem };

export type PublishDraft = {
  version: string;
  notes: string;
  name: string;
  tagline: string;
  category: PublishCategory;
  licence: PublishLicence;
  visibility: PublishVisibility;
  /** Base64 PNG, or null for no icon. */
  iconPng: string | null;
  /** Reason per finding, keyed `path:line`. */
  reasons: Record<string, string>;
};

export type PublishProblems = {
  version?: string;
  notes?: string;
  name?: string;
  tagline?: string;
  /** Per finding key. */
  reasons: Record<string, string>;
  /** Hard findings: nothing in the form fixes them. */
  blocked: boolean;
};

export const LICENCE_LABEL: Record<PublishLicence, string> = {
  MIT: 'MIT',
  'Apache-2.0': 'Apache 2.0',
  'BSD-3-Clause': 'BSD 3-Clause',
  'GPL-3.0-or-later': 'GPL 3.0 or later',
  'AGPL-3.0-or-later': 'AGPL 3.0 or later',
  Unlicense: 'Unlicense'
};

export function isLicence(value: string): value is PublishLicence {
  return PUBLISH_LICENCES.some((licence) => licence === value);
}

export function draftFor(plan: PublishPlanDto): PublishDraft {
  const reasons: Record<string, string> = {};
  for (const finding of plan.waivableFindings) reasons[findingKey(finding)] = finding.reason ?? '';
  return {
    version: plan.suggestedVersion,
    notes: '',
    name: plan.listing.name,
    tagline: plan.listing.tagline,
    category: plan.listing.category,
    licence: isLicence(plan.listing.licence) ? plan.listing.licence : 'MIT',
    visibility: 'public',
    iconPng: null,
    reasons
  };
}

export function problemsOf(draft: PublishDraft, plan: PublishPlanDto): PublishProblems {
  const problems: PublishProblems = { reasons: {}, blocked: plan.blockedFindings.length > 0 };
  const version = versionProblem(draft.version, plan.lastVersion);
  if (version) problems.version = version;
  if (draft.notes.length > PUBLISH_LIMITS.notesChars) problems.notes = `Keep this under ${PUBLISH_LIMITS.notesChars.toLocaleString('en')} characters.`;
  if (plan.firstPublish) {
    if (!draft.name.trim()) problems.name = 'Enter a name.';
    else if (draft.name.trim().length > PUBLISH_LIMITS.nameChars) problems.name = `Keep the name under ${PUBLISH_LIMITS.nameChars} characters.`;
    if (draft.tagline.length > PUBLISH_LIMITS.taglineChars) problems.tagline = `Keep the tagline under ${PUBLISH_LIMITS.taglineChars} characters.`;
  }
  for (const finding of plan.waivableFindings) {
    const key = findingKey(finding);
    const problem = waiverProblem(draft.reasons[key]);
    if (problem) problems.reasons[key] = problem;
  }
  return problems;
}

export function canPublish(problems: PublishProblems): boolean {
  return !problems.blocked && !problems.version && !problems.notes && !problems.name && !problems.tagline &&
    Object.keys(problems.reasons).length === 0;
}

export function formFor(draft: PublishDraft, plan: PublishPlanDto): PublishForm {
  const form: PublishForm = {
    version: draft.version.trim(),
    waivers: plan.waivableFindings.map((finding) => ({
      path: finding.path,
      line: finding.line,
      reason: (draft.reasons[findingKey(finding)] ?? '').trim()
    }))
  };
  const notes = draft.notes.trim();
  if (notes) form.notes = notes;
  if (plan.firstPublish) {
    form.listing = { name: draft.name.trim(), tagline: draft.tagline.trim(), category: draft.category, licence: draft.licence };
    form.visibility = draft.visibility;
    if (draft.iconPng) form.iconPng = draft.iconPng;
  }
  return form;
}

/** The sheet's status line while a publish runs. */
export function progressText(progress: PublishProgress | null): string {
  if (!progress) return 'Waiting for you to confirm…';
  if (progress.phase === 'uploading' && progress.total > 0) return `Uploading ${progress.done} of ${progress.total} ${progress.total === 1 ? 'file' : 'files'}…`;
  if (progress.phase === 'uploading') return 'Uploading…';
  if (progress.phase === 'publishing') return 'Publishing…';
  return 'Checking what the store already has…';
}

/** What a scanner finding is, as a person would say it. */
export function findingLabel(kind: string): string {
  switch (kind) {
    case 'high_entropy': return 'Looks like a key or token';
    case 'pem_private_key': return 'Private key';
    case 'jwt': return 'Access token';
    case 'aws_access_key': return 'AWS access key';
    case 'github_token': return 'GitHub token';
    case 'gitlab_token': return 'GitLab token';
    case 'slack_token': return 'Slack token';
    case 'stripe_key': return 'Stripe key';
    case 'google_api_key': return 'Google API key';
    case 'openai_key': return 'OpenAI API key';
    case 'anthropic_key': return 'Anthropic API key';
    default: return 'Possible secret';
  }
}

export type IconResult = { ok: true; base64: string } | { ok: false; error: string };

const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];

/** Check an icon the way main will: a square PNG of at most 256 KB. */
export function checkIconBytes(bytes: Uint8Array): IconResult {
  if (bytes.length > PUBLISH_LIMITS.iconBytes) return { ok: false, error: 'Choose a PNG of 256 KB or smaller.' };
  if (bytes.length < 24 || !PNG_SIGNATURE.every((value, index) => bytes[index] === value)) return { ok: false, error: 'Choose a PNG file.' };
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(16) !== view.getUint32(20)) return { ok: false, error: 'Choose a square image.' };
  let binary = '';
  for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  return { ok: true, base64: btoa(binary) };
}
