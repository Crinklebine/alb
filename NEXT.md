# Near-term validation work

This is a validation checklist, not a feature roadmap. Current behavior and
limitations are documented in [README.md](README.md).

1. For changes affecting filesystem operations, run the manual native CI matrix
   and record results for the tested revision on Linux, macOS, and Windows.
   Include reprocessing and resume-capacity changes, currently tested only on Linux.
2. Broaden filesystem and network-volume qualification with small disposable
   samples, recording timestamp precision, no-overwrite behavior, interrupted
   builds, and verified resume before attempting full-library trials.
3. Measure large-library inspection, fingerprint-cache reuse, and copy performance
   before choosing optimizations; retain source safety and verification checks.
