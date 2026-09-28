# Project scope and contributor guide

ALB builds an organized output library from one or more source folders while
preserving the sources. Its priorities are conservative handling, traceable
outcomes, and recovery from individual file problems without losing unrelated work.

## Documentation roles

- [README.md](README.md): primary user reference for installation, commands,
  behavior, guarantees, limitations, and examples. Its release information and
  `Cargo.toml` identify the documented release.
- [DECISIONS.md](DECISIONS.md): architectural choices and their rationale.
- [STATUS.md](STATUS.md): current validation evidence and outstanding qualification
  limits, without duplicating the feature reference.
- [NEXT.md](NEXT.md): near-term development or validation work.
- This file: project scope and contributor workflow, not another user manual.

## Scope

ALB organizes supported audio using metadata, performs whole-file exact
deduplication, repairs supported metadata defects on output copies, and optionally
recovers missing identification through AcoustID. Technical validation does not
establish that tags identify the correct performer, recording, or release.

Container checks and encoded-payload preservation support safe copying and tag
repair. They do not establish decoded-audio equivalence or recording quality.
ALB does not transcode audio or use fingerprints to merge similar recordings.
Unknown files remain represented for review.

## Contributor workflow

Keep discovery, inspection, hashing, planning, execution, and platform operations
separate and independently testable. Format-specific details should not leak into
the generic destination planner. Use released registry dependencies and small
synthetic fixtures rather than personal library files.

Preserve tests for source immutability, root isolation, malformed metadata,
exact deduplication, output validation, and verified resume. Before handoff run:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Report which platform was actually tested. A Linux test run is not evidence of
native Windows or macOS behavior. Update user-facing claims in README and record
new architectural rationale in DECISIONS; avoid duplicating release checklists.
