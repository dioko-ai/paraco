# Usability audit — 2026-10-28

## Scope and method

Read README, development/status documents, task/scheduling, local hosting,
marketplace and AI routing contracts, then inspected the corresponding host,
dashboard and Cron implementation. An independent GPT-6 Astra reviewer acted as
product/UX director; a different implementation agent applied the findings from
a principal product manager perspective. The coordinator checked the changes
and real workflows. Four screenshot passes covered desktop (1440×1000) and
mobile (390×844), including viewport captures after saving for review.

The tested product surface is the host dashboard, catalog/setup entry points,
AI settings, and bundled Cron app. This does not establish usability of the
individual third-party catalog apps, live provider accounts, native macOS,
installation on a new machine, or desktop notification delivery.

## Findings and repairs

| Finding | Result |
| --- | --- |
| Mobile catalog text squeezed beside full-height install buttons | Cards stack on mobile; actions keep natural height. Apps/plugins/AI jump links and padded AI controls improve navigation. |
| Publication approval depended on clipped JSON and digests | Visible saved-revision summary explains activation, readable schedule/timezone, timeout, network and inbox behavior. Source diff opens by default; metadata stays in a separate disclosure. Saving moves focus to review. |
| Archived/completed one-off publication consequences were ambiguous | Summary explicitly distinguishes archived jobs, already-admitted one-offs, new overdue one-offs and future recurring runs. Changes to activation state invalidate an open review. |
| Saved drafts falsely reported no unpublished changes | Fixed the database query: unpublished timestamps are NULL, not zero. Regression covers a first draft, publication, and another saved draft. |
| Draft editing and live-job actions competed | Separate published controls from draft saving, use meaningful job titles and explicit inactive/archived state, hide irrelevant delete controls, compact sidebar summaries. |
| First run produced only logs | Starter demonstrates a local reminder with ctx.notify. Guidance explains TypeScript/Web API limits, local inbox destination and automatic versus script-generated notifications. |
| Results and setup were hard to discover | Runs and inbox precede optional AI/history; navigation links lead to runs, editor and dashboard. Runtime diagnostics collapse behind the always-visible machine-running requirement. Empty states explain the next action. |
| AI copy required obsolete manual routing setup | Dashboard settings are the main setup route; explicit file routing remains an advanced option. Updated task documentation to match. |
| Moving inbox content introduced nested cards | Third screenshot review caught it; corrected sibling structure and added an actual-browser DOM assertion. |

## Observed verification

On Linux x86-64 with the repository's pinned compiler/runtime and Playwright:

- Real Chromium and workerd flows: fresh launch, save/review/approval/publication,
  manual success and logs, recurring preview, overdue one-off review, unsaved
  edits against a published job, archive, archived recurring review, automatic
  one-off execution, and review of the already-admitted occurrence.
- No page-script errors in the screenshot passes; final mobile document had no
  horizontal overflow. The catalog regression checks text width, action position
  and button height rather than relying solely on overflow.
- Rust unit tests: 16 library + 117 binary tests passed, including the new draft
  status regression. Socket fixtures require permission to bind local listeners;
  compiler/runtime executables must be on PATH.
- Dashboard/Cron unit and Chromium suites: 43 tests passed with browser cases enabled.
  Added checks cover activation consequences/state invalidation, timezone
  previews, sibling result/inbox/AI/history cards, empty states and archived edits.
- Real dashboard lifecycle smoke passed. AI settings browser fixture passed
  global/default overrides, disabled/inherit modes, local SDK calls, key redaction
  and origin checks. It makes no paid provider requests. The fixture now accepts
  PARACO_BIN like the other host browser tests.
- Formatting, strict Clippy and whitespace checks passed.

Screenshot evidence is retained in this chat's visualization artifacts under
`paraco-ux/round1` through `round4` (about 15 MiB), including initial mobile and
review viewport captures, to support before/after review. Disposable builds, test state and capture scripts were removed after
verification; existing dependencies and unrelated files were preserved.

## Assessment and limits

The independent final screenshot review found no remaining blocker in this
audit scope. The reviewed core journeys are clearer and their visible scheduling
claims now match the implementation exercised here. This remains a code-oriented local
scheduler, with inbox messages rather than native desktop alerts. Human user
research is still needed before claiming broad usefulness or nontechnical ease.
The mobile editor still follows the jobs list and a short runtime reminder; the
editor jump helps, but reducing that initial vertical distance is future polish.
Network grant setup and coding-SDK installation still require technical knowledge.
The individual downloaded apps and public-release gates need their own audits.
