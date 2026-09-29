# Architectural decisions

This document describes current design choices and their rationale. It consolidates
superseded implementation checkpoints; repository history retains those checkpoints.
User-visible commands and detailed behavior belong in [README.md](README.md).

## Source-preserving library builder

ALB organizes a separate library rather than editing the input in place. Sources
are opened read-only; reads may still update operating-system access times.
Canonical equal/nested-root rejection, identity checks, and source-snapshot tests
protect this boundary. Existing outputs are never retagged or overwritten, since
an output may be hardlinked to a source.

Library writes and audits belong under output. Fingerprint cache state belongs in
the platform's ALB configuration directory. That cache location is not checked
against input roots; keep it outside source trees or disable the cache. Scan and
dry-run do not write library files, but may update that cache.

## Detected formats and a generic catalog

Extensions are discovery hints. `media` identifies supported content and checks
container boundaries; detected type determines the output group. This supersedes
the original extension-only policy and prevents mislabeled files from entering
the wrong format folder. Unknown/non-audio data remains preserved for review.
These checks do not decode every sample or establish audio quality.

`inspection` uses Lofty for FLAC, M4A, MP3, OGG, and WAV metadata. Primary fields
and secondary tags populate a generic catalog with source identity and warnings.
Tolerant reads can salvage fields; strict output validation is a separate gate.
The planner consumes generic metadata and type rather than codec-specific fields.

## Technical metadata validation, not factual identification

New normal-library copies require usable Artist and Title and strict metadata
read-back. Repair takes place on a private output copy. The encoded payload and
readable artwork are checked so a tag repair cannot silently authorize audio loss.
Unsupported repairs retain the original bytes in Problem Files.

This establishes technical compliance with ALB's validation rules, not factual
correctness of Artist, Title, or Album. Conflicts remain visible in reports rather
than being resolved through speculative canonicalization. Missing optional album
metadata uses Loose Tracks; ALB does not infer a commercial single.

AcoustID is an optional missing-Artist/Title fallback, not a continuous retagging
service. Usable embedded fields take precedence; missing Album alone does not
trigger recovery. Cached unexpired matches can recover fields without a key;
new network lookups require one. Provenance is recorded and recovered copies use
Fingerprinted routing. Lookup does not submit database contributions.

## Whole-file exact deduplication

Streaming BLAKE3 hashes and source stamps supply evidence for grouping within each
detected supported format. This compares complete source bytes, including tags;
it is not audio-equivalence, fuzzy matching, or a quality judgment. AcoustID
fingerprints are not deduplication evidence.

A deterministic eligible source represents an exact duplicate group. Omission is
allowed only after the representative output is verified, with duplicate source
evidence rechecked. UNKNOWN and unresolved problem files retain independent copies.
Different-byte files targeting the same name receive disambiguated names, not an
arbitrary winning copy. Collision naming assumes stable roots and group membership.

## Names and per-file isolation

Portable sanitization, bounded names, and deterministic folder spelling reduce
cross-filesystem collisions. These rules are conservative rather than a complete
model of filesystem collation. Album Artist may choose an album folder; it is not
silently substituted for a missing track Artist.

Problem routing retains source-relative folders and the original basename where
safe, shortening or disambiguating only as needed. Sidecars put the explanation
first and retain source paths, outcome, and original times. A matching ALB sidecar
is required before removing old Problem Files wrappers during reprocessing.
An input with a genuine build-report header directly under `_ALB` is treated as
previous ALB output. Its unknown-file paths lose repeated leading UNKNOWN wrappers;
recognized old audit reports and problem explanations are copied unchanged under
`_ALB/Previous Reports`. This retains audit history while new explanations stay
beside current problem copies. Report recognition combines location and content,
so ordinary text files are preserved through the unknown-file planner.

Per-file failures do not stop unrelated work. An unreadable or unverifiable source
is honestly reported as NOT COPIED when reporting is possible. Root isolation,
output-lock, and audit failures remain fatal: continuing could violate source
safety or produce misleading success records.

## Private staging, publication, and verified resume

Execution exclusively creates partial files, bounds transfer against expected
source evidence, syncs and rereads copied bytes, and checks source identity.
Metadata changes occur only on these private copies. Publication uses native
no-replace operations; final bytes are checked against the resulting evidence.
A staged partial alone never authorizes duplicate omission.

Resume reconstructs the current plan and verifies existing outputs rather than
trusting prior reports. For normalized/recovered output, it prepares a fresh
comparison copy and compares final bytes. Matching output is reused; mismatches
are not overwritten. Existing timestamps are checked, not changed.

Old partials remain untouched. Current-run failed partials are removed only after
a verified Problem Files fallback succeeds; failed fallback retains them. New
comparison partials are removed after successful reuse. Audit records preserve
source and output evidence but are not a trusted resume manifest.

## Native platform boundary and bounded guarantees

`src/platform/` separates shared execution policy from filesystem operations:

- Linux uses directory handles, `openat2` with beneath/no-symlink/no-mount-crossing
  restrictions, directory locking, and `renameat2` no-replace publication.
- macOS uses single-component no-follow `openat`, device checks, directory locking,
  and exclusive rename publication.
- Windows retains ancestor handles without delete sharing, rejects reparse points,
  locks output through an exclusive lock-file handle, and requests no-replace,
  write-through publication.

Source identity comes from handles, including Windows volume/file identity and
change time. Stable trees under user control are required. Hostile same-user
mutation, arbitrary mount changes, and relocation of held Unix directories are
outside the supported model. Snapshot checks alone are not atomic protection.

Unix syncs files and parent directories. Windows flushes files and requests
write-through publication but has no general directory-flush operation here.
Crash durability depends on the filesystem; unsupported required operations fail
rather than falling back to overwriting or following links.

## Archival times and capacity

New copies restore and verify modification time; macOS/Windows also restore
available creation time. Linux birth time is archived rather than reset. Reports
use readable UTC timestamps with nanosecond precision and explicit unavailable
values, never substitute ctime, and retain each duplicate source's own times.

Capacity checks query the output filesystem or its nearest existing ancestor,
include an allowance, and repeat before copying. They neither reserve capacity
nor fully model quotas, compression, shared blocks, or concurrent disk use.
Resume comparison copies also require scratch space.
The current preflight sums tagged-output comparison sizes instead of modeling
their sequential lifetime. This conservative overestimate can block otherwise
feasible resumes; correcting it is tracked in NEXT.

## Dependencies and validation

Use released registry dependencies, pinned in Cargo.toml/Cargo.lock. Lofty's
metadata readers/writers and the bounded media checks support organization;
there is no custom full audio decoder or transcoding pipeline.

The manual native CI matrix uses production default features for tests, Clippy,
and release-build verification. One-day artifacts are diagnostic build outputs;
crates.io is the distribution channel. Contributor checks are listed in PROJECT.
