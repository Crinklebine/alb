# Validation status

This file records validation evidence and its limits. See [README.md](README.md)
for implemented behavior and [NEXT.md](NEXT.md) for follow-up work.

## Current evidence

- On 2026-09-29, the reprocessing changes passed the full Linux test suite,
  formatting, and Clippy with warnings denied. Regression coverage includes
  repeated builds, report archiving, old UNKNOWN-wrapper cleanup, ordinary text
  preservation, verified resume, and unchanged source contents. At that point,
  the executable reported version 0.4.14.
- The subsequent resume-capacity fix passed all 174 tests, formatting, and Clippy
  with warnings denied on Linux. New regressions use real build/resume plans and
  simulate a free-space threshold where the largest comparison fits but their sum
  does not. They also cover new files, verified unchanged outputs, duplicates,
  destinations removed after planning, and full allowances for problem copies.
  The executable was rebuilt after the patch-version bump and reports 0.4.15.
- User trials have exercised the same library data on Linux, macOS, and Windows.
  These trials establish practical coverage, not qualification of every filesystem.
- Automated tests cover all five formats, source preservation, malformed tags,
  problem routing, exact duplicates, resume verification, and filesystem safety.
  Platform-specific tests run only on their applicable operating system.
- The native GitHub Actions workflow is manually dispatched. Its configuration
  runs tests, formatting, Clippy, and release-build checks on all three platforms;
  configuration alone is not evidence that a particular revision passed.

## Qualification limits

There is no comprehensive qualification matrix for network volumes, filesystem
collation, timestamp precision, or crash recovery. Local test results must not be
presented as guarantees for every APFS, NTFS, or Linux filesystem configuration.
The supported operating model and durability limits are documented in README.
The reprocessing and resume-capacity changes have not been validated natively on
macOS or Windows. Limited free space in the capacity regression is simulated;
it is not qualification of a physically full filesystem or quota enforcement.
