# AGENTS.md — MuDraft working rules

Product contract: `PROJECT_SPEC.md`. Current state: `BUILD_STATUS.md`. This repo is standalone; never modify FDraft or assume its source is available.

## Scope

- Implement only the supplied step, in small coherent changes.
- Preserve working code. No unrelated rewrites, speculative frameworks, or unrequested features.
- Read `BUILD_STATUS.md` and the relevant spec sections first, then inspect only affected code. Do not repeatedly dump the repository or dependency files.

## Docs

- Keep this file and `BUILD_STATUS.md` under 250 words each.
- `BUILD_STATUS.md` records the completed step, checks actually run, blockers, and next step. Replace stale notes; never accumulate transcripts.

## Dependencies

- Use current supported stable releases, pin the toolchain, and commit lockfiles.
- Verify unfamiliar APIs against official documentation.
- Do not upgrade unrelated dependencies mid-build.

## Quality

- Add focused tests with each feature and run the affected checks.
- Never disable failing checks or substitute production mock data.
- Keep fixtures and test bridges out of release builds.

## User data

- Preserve user data. Validate at the native (Rust) boundary and use real transactions.
- Never silently reset storage after an error.

## Finishing a step

- Final report of at most 180 words: changes, checks actually run, remaining blockers, next step.
- Do not print entire files. Stop after the step.
