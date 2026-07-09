---
name: new-feature
description: Scaffold a new paired backlog user story + TDR design doc, following the docs/backlog and docs/tdr numbering and template conventions. Use when starting work on a new feature or endpoint.
disable-model-invocation: true
---

Create a new numbered feature doc pair for: $ARGUMENTS

1. Determine the next number: look at existing files in `docs/backlog/` and `docs/tdr/`, find the highest `NNN_` prefix across both directories, and increment by 1 (zero-padded to 3 digits, e.g. `001`).
2. Slugify the feature name from $ARGUMENTS (lowercase, spaces/punctuation to underscores) for use in filenames.
3. Create `docs/backlog/NNN_<slug>.md` matching the structure of `docs/backlog/000_landing_page.md`:
   - `# User Story: <Title>`
   - `## 1. User Value Statement` — As a **<role>**, I want to **<capability>**, So that **<benefit>**.
   - `## 2. Strict Acceptance Criteria` — numbered `AC-N` bullets, specific and testable.
4. Create `docs/tdr/NNN_<slug>_design.md` matching the structure of `docs/tdr/000_landing_page_design.md`:
   - `# TDR NNN: <Title>`
   - `## 1. Context & Architectural Requirements`
   - `## 2. Alternatives Evaluated` — at least two `### Alternative` subsections with Pros/Cons
   - `## 3. Structural Decision` — which alternative was chosen and why
   - `## 4. OpenTelemetry Implications` — how the feature will be instrumented per CLAUDE.md's telemetry rules
5. Infer the user story and TDR content from $ARGUMENTS and the project's architecture rules in CLAUDE.md (multi-tenancy, OCR, blob storage, telemetry) as relevant to the feature. Ask the user for clarification only if the feature description is too vague to draft acceptance criteria.
6. Report the two file paths created.
