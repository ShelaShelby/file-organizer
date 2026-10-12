# Report – Stage 2

## Built
- `core/` (no GUI), new modules: `tree` (walk Downloads/Desktop/Documents WITH sub-folders, stage-1 skip
  rules; links/junctions, hidden/system and protected folders are never entered), `dupes` (size → BLAKE3 of
  first 16 KB → full BLAKE3; min size setting, empty files always ignored), `bigold` (top 200 biggest /
  oldest by last-changed), `recycle` (Recycle Bin), `remove` (re-check + journal).
- Recycle Bin: Windows `IFileOperation` with `FOFX_RECYCLEONDELETE`; a progress sink cancels any item in
  `PreDeleteItem` that Windows would delete for good (no `TSF_DELETE_RECYCLE_IF_POSSIBLE`), so such files
  are left and reported. Nothing in the app can delete permanently. `Recycler` trait lets tests swap it.
- Before each removal the file is checked again: inside a root, no junction/link/hidden folder on the way,
  skip rules, not locked, same size and last-changed time as at the scan. Otherwise skipped.
- Duplicates: a group is skipped if it has no kept copy, or a kept copy is gone/changed (at least one copy
  always stays). Confirm is disabled while any ticked group keeps no copy.
- Journal: `recycle` records (old path, place in the Recycle Bin). History shows removals with old paths
  and "restore them from the Recycle Bin".
- Settings: big-file MB (500), old days (365), min duplicate KB (1), plus the 7-day rule. Per-value fallback
  to defaults; an unreadable settings.json is never overwritten (Save refuses).
- GUI: tabs Sort / Duplicates / Big and old, Settings panel (top right). Scans run on a background thread
  with file count, seconds and Cancel. Duplicates: per-file "Keep", per-group tick, space freed, Confirm
  dialog listing every file that will go. Big and old: Open folder (Explorer /select), Recycle (asks first).
- New dependencies: `blake3`, `windows` + `windows-core` (Windows only, for IFileOperation).

## Tests
- 64 tests on Windows in Actions, all passing: 35 stage 1 (`rules.rs`) + 29 stage 2 (`stage2.rs`):
  content-not-name, same size/different content (also differing only after the first block), empty/tiny,
  sub-folders, oldest kept by default, change kept copy / untick group, zero-copy groups refused,
  changed-after-scan skipped (size and time), kept copy changed → group left, recycler failure leaves file,
  hidden/system files+folders, junctions, symlinks, locked, temp/shortcut/desktop.ini, outside-root and
  `..` refused, cancel, big/old thresholds + settings + cap, settings fallback / no overwrite,
  and one real Windows Recycle Bin test.

## Size
- Exe 5,888,512 bytes (~5.6 MB; stage 1 was ~5.5 MB).

## Known gaps
- No Undo for removals: restoring needs undocumented Recycle Bin internals; use Recycle Bin → Restore
  (History shows the old paths).
- Duplicates are checked by size + last-changed time at confirm, not re-hashed.
- Online-only OneDrive files are skipped (not downloaded to hash them).
- GUI itself is not covered by automated tests (only the library is).
