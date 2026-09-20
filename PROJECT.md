# ALB — Audio Library Builder

## Product goal

Sort and rebuild a large audio library into a separate organized output library.
ALB is a sorter, not an audio analyzer. V1 classifies files by their final filename
extension only, case-insensitively:

| Extension | Output group |
| --- | --- |
| .flac | FLAC |
| .m4a | M4A |
| .mp3 | MP3 |
| .ogg | OGG |
| .wav | WAV |
| Anything else | UNKNOWN |

Do not inspect M4A or OGG internals to distinguish codecs for sorting. Do not
expand supported extensions until real UNKNOWN contents justify the change.
Preserve unknown files and report them. Use output `_ALB/` for build reports; future versioned manifests may live there.

## Hard safety invariants

- Input is strictly read-only: never rename, move, delete, retag, intentionally
  change timestamps, or create cache/state/report files inside it.
- All library writes belong to output; explicit project-state files must be
  outside input. Reading may cause operating-system access-time updates.
- Reject equal input/output roots and nesting in either direction, including
  canonical aliases and roots with missing output components.
- Do not follow source symlinks. Preserve discovered regular files, including
  unknown types and files with unreadable metadata; report errors and continue.
- Dry-run never creates directories, copies files or writes reports.
- Execution must copy, never move, avoid overwrites, verify copies, and
  distinguish incomplete output after interruption. No unsafe source cleanup.

## Architecture

Read-only scan -> extension classification -> basic metadata -> generic catalog
-> normalization -> exact same-type dedupe -> destination planning -> dry-run and
reporting -> safe copy -> output library.

Format adapters populate generic fields: source path, file type, artist, album
artist, album, title, track/disc numbers, optional duration and source identity.
Hash evidence can be stored separately. Planning must not depend on codec internals.
Keep stages separate, deterministic and independently testable.

Prefer album artist for compilation folders, then artist. Organize tagged files
by type/artist/album. Avoid guessing weak metadata or declaring singles just
because a folder has one track. Missing/blank albums with usable artist/title use
TYPE/Artist/Loose Tracks/Title - Artist.extension without track/disc numbering.
Other fallback locations preserve source-relative paths.
Normalize names conservatively; report ambiguous destinations, never pick collision
winners silently. UNKNOWN files must remain represented even when naming fails.

## V1 deduplication and scope

Use BLAKE3 whole-file hashes only. Compare within the same extension-defined file
type; retain copies across different types. Report evidence and deterministic
representative selection before introducing omission during copying.

No decoded-audio hashes, fingerprinting, silence/spectral analysis, remaster or
transcode detection, codec forensics, online metadata or audio editing. FLAC needs
only extension classification, basic sorting tags, optional duration and exact
whole-file hashing. Successful metadata reading is not an audio-validity claim.
Use released dependencies; do not fork or patch dependencies for analysis.

## Resumable development

Work in modest tested increments. Repository notes carry context between sessions:
PROJECT.md for goals, STATUS.md for actual capabilities/limits, DECISIONS.md for
architecture choices, NEXT.md for the next few ordered steps. These notes do not
substitute for a future resumable build manifest.

At session start read these files and relevant code. Preserve source-immutability,
root-safety, classification, generic catalog, basic metadata, malformed-file and
exact-hash tests. Use small original synthetic fixtures. Before handoff run
`cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features`;
update the notes and leave no misleading claims of unfinished features.

## First user-testable version gate

Basic sorting-metadata support for all five supported formats (FLAC, M4A, MP3,
OGG and WAV) is mandatory before user/library testing. Fallback paths remain for
missing or unreadable tags, not as a substitute for implementing an adapter.
Continue automated synthetic-fixture testing during development. Safe build
execution and end-to-end automated checks are also required before user testing.

## Execution platforms and recovery

Linux, macOS and Windows have native filesystem backends behind a shared interface. Require
directory-handle operations and atomic no-replace publication, failing closed if
unsupported. Stable trees under user control are required; hostile directory
relocation or mount manipulation is outside the supported operating model.

Durable audit reports record evidence and verified outcomes. Resume reconstructs
the plan and rehashes existing outputs rather than trusting old reports. Never
overwrite mismatched destinations or reuse/truncate old partials. Resume may
restart an incomplete copy under a new partial name.

## User requirement — file problems must not stop unrelated work

Route per-file exceptions into output Problem Files/<Problem Type>/ and write
detailed adjacent text explanations. Continue unaffected work. Unreadable or
unverifiable files get honest NOT COPIED explanations when output is writable.
Keep source immutability, root isolation, no-overwrite and copy verification.
Show clean single-line terminal stage/count/activity progress; redirected logs
use bounded stage summaries. This supersedes earlier all-plan blocking rules.

## Archival requirements

Preserve original source modification time on new copies and archive original
per-source creation/modified times. Restore filesystem birth time on macOS/Windows when supplied; Linux birth time
cannot be restored through ordinary APIs, so retain it explicitly in durable audit
records on all platforms and mark unavailable values honestly. Never mistake ctime for creation time or mutate an
existing output inode that may be hardlinked to source.
Check destination available capacity before writes, with overhead allowance and
per-copy rechecks. Initial insufficient capacity is a global safety stop.
