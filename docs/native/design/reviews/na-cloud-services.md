# Design review: `feat/na-cloud-services`

Reviewer: design lead · 2026-11-01 · Commits reviewed: `00a61ff61`, `d1fec43c0`, `beb097746`
Scope: user-visible consequences of the services crate (library/recents,
updater, fonts, diagnostics, entry points, Store metadata). The lane draws no
UI; this review fixes the data contracts so the UI lanes can follow
[language.md](../language.md). Nothing here blocks the lane's plumbing merge
except item 1.

## Must-fix

1. **No developer strings reach the UI.** Services return `anyhow` messages
   such as `Store requires a credential-free HTTPS origin`, `Store response too
   large`, `expected absolute .pmv file`, `duplicate needs a new id`. These are
   log text. Every error a UI can show needs a typed code (e.g.
   `ServiceError::{Offline, NotFound, TooLarge, InvalidProject, PermissionDenied,
   DiskFull, Unknown}`) carried alongside the message, and the UI owns the
   sentence-case copy. Example copy: InvalidProject → "“Launch Film” can’t be
   opened because it isn’t a Powermove project."
2. **Library vocabulary is Mac vocabulary.** `LibraryAction::Archive/Restore`
   with `ProjectEntry.trashed` must surface as **Move to Trash** / **Put Back**
   (menu items, Title Case) and the list filter as **Recently Deleted**, never
   "Archive". Keep the internal names if they are durable formats; add a
   comment pointing to these labels.

## Should-fix

3. **Project Library rows need display data.** `ProjectEntry` has only id,
   name, modified and trashed. The Project Library window (replacing
   Electron's Home screen) shows a thumbnail, duration, resolution and frame
   rate like the v1.2.0 projects grid. Add `thumbnail: Option<AssetId>`,
   `duration_ms`, `width`, `height`, `fps` (or a `summary` struct the kernel
   fills on save). Dates display with the system relative formatter
   ("Today, 11:42", "Yesterday").
4. **Update notes are untrusted rich text.** `FeedMetadata.notes` (≤ 64 KiB)
   must be documented as Markdown (or plain text) and rendered in the
   standard update sheet with the language's `reading` style, links opening
   in the browser. Add `name` (e.g. "Powermove 2.1") and `size_bytes` for the
   sheet: title "A New Version of Powermove Is Available", buttons
   `Install Update` (accent) / `Remind Me Later` / `Skip This Version` —
   the Sparkle/macOS convention.
5. **Font catalog groups faces.** `families()` returns family names only. The
   font pop-up shows families with their faces in a submenu, each family name
   drawn in its own face (system font panel behaviour). Provide
   `faces(family) -> Vec<Face{name, weight, italic}>` from the same CoreText
   broker; the inspector's Weight row reads these.

## Nice-to-have

6. Diagnostics: crash records should carry a user-facing `recovered_document`
   name so the next launch can say "Powermove recovered “Launch Film” after
   an unexpected quit." (sentence case, `reading` style, one Show button).
7. Entry queue: when a `.pmv` open fails, keep the path so the UI can offer
   "Show in Finder" in the error alert.

## Re-review — 2026-11-01

Commits: `468f2e0a2` … `3c68b9c19` on `feat/na-cloud-services`.
**Verdict: APPROVE — merge `origin/feat/na-cloud-services` as is.** (Its
`pm-services` entry in `apps/native/Cargo.lock` is still owed by the merge.)

| # | Item | Status |
|---|---|---|
| 1 | No developer strings | Done. `ServiceError { code, diagnostic }`; Display/Debug are log text only. |
| 2 | Mac library vocabulary | Done. `Move to Trash`, `Put Back`, `Recently Deleted` exported; internal names documented. |
| 3 | Library row facts | Done. `ProjectSummary` (thumbnail, duration, size, exact fps); dates stay instants for the system formatter. |
| 4 | Update sheet | Done. `name`, `size_bytes`, notes as untrusted Markdown ≤ 64 KiB; title and three button labels verbatim. |
| 5 | Font faces | Done. `faces(family)` from the same broker. |
| 6 | Recovered document name | Done (`CrashRecord.recovered_document`, omitted from exports). |
| 7 | Failed-open path | Done (`failed_file()`, `Show in Finder`). |

Copy for UI lanes, keyed by `ServiceErrorCode` (sentence case; `“%@”` is the
document or item name when known, otherwise drop the clause):

| Code | Alert message | Informative text |
|---|---|---|
| Offline | Powermove can’t connect. | Check your internet connection and try again. |
| NotFound | “Launch Film” can’t be found. | It may have been moved or deleted. |
| TooLarge | “Launch Film” is too large to open. | — |
| InvalidProject | “Launch Film” can’t be opened because it isn’t a Powermove project. | — |
| PermissionDenied | You don’t have permission to open “Launch Film”. | Check its sharing and permissions in Finder. |
| DiskFull | There isn’t enough disk space. | Free some space and try again. |
| InvalidInput / InvalidData | “Launch Film” couldn’t be read. | The file may be damaged. |
| Unavailable / Unknown | The operation couldn’t be completed. | Try again. If it keeps happening, choose Help ▸ Report a Problem. |
