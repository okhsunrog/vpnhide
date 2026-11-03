# Maintenance lock lifetime under fork — 3 November 2026

Engineering fix after `122d1728b145072922e50386fd9e585cdbac382b`.
Evidence lives under `vpnhide-build-evidence/maintenance-lock-lifetime/` in the
execution workspace. No device or manager operation was run.

A parallel Core test exposed a real lifetime boundary: an accepted-floor lease
had been dropped, but a new independent lease still returned Busy. Two isolated
regressions reproduced the same problem for floor and staging. A deliberately
paused fork child retains the open-file description before CLOEXEC can run;
ordinary File drop in the parent therefore does not end the advisory lock.

Core now uses one private `FileLock` guard for instance, accepted-floor and staging
exclusion. Admission stays nonblocking and independent writers remain excluded
while the owner lives. The owning process explicitly unlocks at scope end, without
deleting/replacing the lock inode. A child's copy closes its inherited descriptor
without unlocking its live parent. Existing inode, owner, mode, signature, floor
and durability checks remain in their original callers.

The two owner-release regressions failed before the fix and pass afterwards.
A third regression drops the inherited guard in a child and requires the parent
still to exclude a contender. Existing instance-lock/floor/staging tests remain.
The final integrated Rust and strict host/ARM gates include these cases. The
snapshot-codec migration was also present in the working tree during integration;
this lock primitive does not depend on that migration. Exact changed-source hashes
and the checked context are retained with the evidence.

RPC31, journal14, diagnostic9 and debug schema11 are unchanged. This neither
retries a manager effect nor changes backend selection, exposure or fallback.
Physical Android/manager acceptance remains separate.
