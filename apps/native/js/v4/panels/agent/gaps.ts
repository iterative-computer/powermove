/** Explicit, inspectable blockers. Metadata is diagnostic data, never a renderer API. */
export const gaps = {
  composer:
    "Native multiline composer, IME-aware default cancellation, inline tokens and pasteboard driver are absent from the UI kit.",
  richText:
    "Selectable Markdown, code/table scrolling, thumbnails and copy affordances are absent from the UI kit.",
  variableList:
    "The v4 virtual-list supports fixed itemSize only; variable heights, measurement cache, id anchors, AXRowCount and scroll-to-row need a host/kit API.",
  preview:
    "Isolated fixed-aspect visual and fitted-attachment preview controls and their resource leases are absent from the UI kit.",
  threadPicker:
    "Thread picker search, unread facts, confirmed archival and windowed thread publications need the model/host adapter.",
  accessMenu:
    "Menu subtitles, warning symbol and native Full Access consent need the host adapter.",
  launcher:
    "Toolbar symbol/ring/dot/badge rendering has no UI-kit contract; launcher facts are metadata only.",
  shell:
    "Popover size math, pin/drag/dock effects, layout Undo, visibility and focus restoration belong to pm-shell.",
  geometry:
    "Token arithmetic, wrapping, agent-specific disclosure/button slots and EmptyState display typography require UI-kit support; composer_max_lines is not in pm-design yet.",
  accessibility:
    "Native live announcements, scroll actions, AXRowCount, text hints and group roles need host projection support.",
} as const;
export type Gap = keyof typeof gaps;
// DESIGN-TODO(opus): §3 anatomy depicts a search header button, but §3's header
// table and §6's exhaustive tab order omit it. Placeholder omits search pending resolution.
// DESIGN-TODO(opus): specify approved tokens for §3/§4 literal-only strokes,
// attention/unread dots, code padding and below-300-pt model collapse. No values substituted.
// DESIGN-TODO(opus): model question choices require explicit Answer and never
// submit on choice; reconcile §3's default-choice buttons with model/v1.2.0 semantics.

// DESIGN-TODO(opus): the model restores interrupted reply questions as `closed`;
// the spec names only Answered/Skipped. Closed questions remain plain labelled
// placeholders without submission controls until their copy/state is specified.
