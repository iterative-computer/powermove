# Design review: `feat/na-cloud-storehost` (Store permission prompts)

Reviewer: design lead · 2026-11-01 · Commits reviewed: `776d45670`, `c6427adf8`
Scope: the shape and copy of permission prompts (`store-host/src/permission.rs`,
`docs/native/store-host.md` §Permission prompts). The lane is plumbing; the
prompt structure it returns decides what the UI can say, so it is reviewed
here. Read against [language.md](../language.md) §9 (sheets and alerts) and §14.

**Result: changes required before a UI binds to it.** The security model is
sound (publisher/revision binding, durable CAS before grant, Full Access never
downgraded). The prompt **data** is too thin to present honestly.

## Must-fix

1. **One consent per install, not one prompt per permission.** `request`
   yields a prompt for a single `Permission`; an extension declaring network +
   clipboard + project:write would show three prompts in a row. v1.2.0
   disclosed all permissions together on the Store page (`permissionLines`,
   Full Access first). Add `PermissionRequest { permissions: Vec<Permission>, .. }`
   that covers every *undecided* declared permission for the active release,
   and a `respond_all` that records each decision in one CAS. Single-permission
   prompts remain only for a permission added by an update.
2. **Prompts need display data.** `PermissionPrompt` carries ids only. Add
   `extension_name` (display name from the signed manifest), `publisher_name`,
   `publisher_verified: bool`, `icon` (bundle asset id) and
   `reason: Option<String>` (author-supplied purpose string, ≤ 140 chars,
   plain text, from the manifest like `NS…UsageDescription`). Never show the
   raw extension id or bundle hash as the primary label.
3. **Full Access is an alert, the rest is a sheet.** `requires_trusted_realm`
   must drive two different presentations:
   - **Isolated permissions** → a window-modal **sheet** titled
     `Allow “Glass Blur” To:` listing one row per permission (SF Symbol +
     Title Case name + sentence-case detail), buttons `Don’t Allow` and
     `Allow` (accent, default). Not a centred dialog.
   - **Full Access** → an `NSAlert` with the warning symbol, the exact v1.2.0
     copy: message `Give Glass Blur full access to Powermove?`, informative
     text `It will run with the same access as the app: your projects, files
     you open, the network, and other extensions’ values. Only trust
     extensions from people you know. You can revoke this from the Library.`
     Buttons `Trust` and `Cancel`; **Cancel is the default** (v1.2.0
     `trustDialog`, `defaultId: 1`).
4. **Permission copy table** (single source; Title Case name, sentence-case
   detail; same order as v1.2.0, Full Access first):

   | Permission | Symbol | Name | Detail |
   |---|---|---|---|
   | `full-access` | `exclamationmark.shield` | Full Access | Runs with the same access as Powermove. |
   | `project:write` | `pencil` | Edit Your Project | Can change layers, keyframes and media in open projects. |
   | `assets` | `photo.on.rectangle` | Import Files | Can add files you choose to your project. |
   | `network` | `network` | Use the Network | Can load images, fonts and data from the internet. |
   | `clipboard` | `doc.on.clipboard` | Use the Clipboard | Can copy to and read from the clipboard when you ask it to. |

   The Store page keeps v1.2.0's disclosure labels (`Uses the network`,
   `Needs full access to Powermove`, …); the sheet uses the table above.

## Should-fix

5. **Denied is not silent.** `Admission::Denied` from a stored denial gives
   the extension an error but the user no path back. Expose the stored
   decisions per extension so Settings ▸ Extensions can show each permission
   as a switch (`Allow`), with revoke going through the existing `revoke`.
6. **Stale prompts.** When `respond` returns `Stale` (the extension updated
   while the sheet was open), the UI must dismiss the sheet and re-request,
   not show an error. Document this as the contract.
7. **Prompt identity for VoiceOver/MCP.** Keep `id` stable across re-requests
   with identical inputs (it is: SHA-256 of the binding) and expose it as the
   sheet's accessibility identifier so agents and UI tests can target it.

## Nice-to-have

8. Remember the anchoring: if the request comes from a Store view panel, the
   sheet attaches to that panel's window; from the agent/MCP, show an inline
   agent question card first, then the sheet when the user accepts.
9. A `learn_more_url` (https only, Store-origin) shown as a link-style button
   in the sheet footer.

## Re-review — 2026-11-01

Commits: `cc65c000c` … `3b8e237c9` on `feat/na-cloud-storehost`.
**Verdict: APPROVE with a design fix — merge
`origin/feat/na-design-fix-storehost` (`651740fea`, based on the lane
branch), not the lane branch itself.**

| # | Item | Status |
|---|---|---|
| 1 | One consent per install | Done. `PermissionRequest.permissions` covers every undecided declared permission; `respond_all` records them in one CAS; updates prompt only for added permissions. |
| 2 | Display data | Done. `extension_name` (signed manifest), host-provisioned `publisher_name` / `publisher_verified` (a manifest cannot claim them), bundle `icon` asset id, `reason` ≤ 140 chars, plain text. |
| 3 | Sheet vs alert | Done. `presentation()` → `WindowModalSheet` (`Allow “Glass Blur” To:`, `Don’t Allow` / `Allow` accent default) or `NsAlertWarning` (exact v1.2.0 message and informative text, `Trust` / `Cancel`, Cancel default). |
| 4 | Copy table | Done verbatim, in disclosure order; Store labels match v1.2.0 `PERMISSION_LABEL`. |
| 5 | Denied is not silent | Done. `stored_decisions` with `active` flags; `SETTINGS_ALLOW_LABEL`. |
| 6 | Stale prompts | Done and documented: dismiss and re-request, no error. |
| 7 | Prompt identity | Done. `accessibility_identifier` = request id. |
| 8–9 | Anchoring, Learn More | Deferred (no owner/endpoint yet). Acceptable. |

The lane left a `DESIGN-TODO(opus)` for records without a signed name,
publisher name or icon. Specified and implemented on the fix branch:

| Missing | Shown |
|---|---|
| Extension name | Sheet `Allow This Extension To:`; alert `Give this extension full access to Powermove?` |
| Publisher name | `From an unidentified developer` (no badge) |
| Icon | SF Symbol `puzzlepiece.extension`, `secondary` |

A named publisher reads `From Example Publisher` (+ `checkmark.seal.fill`
in `accent_text` when the host verified it) or `From Example Publisher ·
Unverified`. `presentation().title` is now always a `String`; new fields
`publisher_line`, `publisher_badge`, `icon_symbol`. 88 contract tests pass
(isolated manifest), clippy `-D warnings` clean.
