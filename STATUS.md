# Current status — ALB 0.2.0

## Cross-platform implementation

- Shared scan/classification, five-format metadata, exact dedupe, destination
  planning, problem isolation, verification, reports, resume and space policy.
- src/platform provides the OS boundary: Directory, source handles/identity,
  link detection, timestamps, free-space access and terminal capabilities.
- Unix backend retains Linux openat2 protection; macOS uses single-component
  no-follow openat, device checks and native exclusive rename.
- Windows pins every ancestor without delete sharing, rejects reparse points,
  publishes with no-replace write-through moves and uses an exclusive lock file.
- Source stamps use handle-based identity on all platforms (Windows volume/file
  identity and change time). Discovery also skips Windows junctions/reparse points.
- New copies restore mtime everywhere and available birth time on macOS/Windows.
  Audits retain original times everywhere. Linux birth time remains audit-only.
- Platform-neutral executor is enabled on all three systems; no Linux-only stub.

## Validation in progress

Linux: 108 tests pass; formatting and Clippy with warnings denied pass.
Windows and macOS: all-target compile/Clippy checks pass from Linux using the pure
BLAKE3 feature for cross checks only. Native CI tests production default features,
creates real files and runs filesystem safety tests on all three operating systems.
Native CI outcome must be confirmed before final handoff.

## Operating limits

Stable source/output trees under user control are required. Unix directory
relocation by hostile processes is outside the supported model. Windows blocks
ancestor renaming while held. All three refuse links/reparse traversal during
execution. Windows cannot generally flush directory handles; publication requests
write-through and file data is synced, with filesystem-dependent crash durability.

Unknown data and problem files remain independent. Unreadable files cannot be
invented: report NOT COPIED and continue unrelated work. Root, output-lock and
audit failures remain fatal. Native timestamp precision is required. Resume does
not mutate existing timestamps and retains old partials.
