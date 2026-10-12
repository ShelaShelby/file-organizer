# Report – Stage 1

## Built
- `core/` (crate `organizer-core`, no GUI): Known Folder lookup (Downloads/Desktop/Documents, OneDrive-safe),
  protected-path guard, file safety checks, scan → preview plan, confirmed execution, JSONL journal, undo
  (last batch or any batch from history), settings.json with the extension map and day count.
- `src/` (eframe/egui): dark window with Scan, preview table (checkbox, file, from, to, size), Confirm,
  Undo last, History list with per-batch Undo, summary after each batch (moved / skipped / errors + details).
- Moves use `MoveFileExW` without replace (cannot overwrite); names get " (2)", " (3)" on collision.
- GitHub Actions (windows-latest): release build, tests, `file-organizer.zip` (exe + README.txt),
  "latest" release recreated on every push to main, targeting that commit.

## Tests
- 35 tests on Windows (`core/tests/rules.rs`), all passing in Actions: type + month layout, Desktop →
  Documents\From Desktop, age rules (7 days / 2 minutes / setting), temp, downloading, shortcuts,
  desktop.ini, hidden, system, locked files (at scan and at confirm), junctions, symlinks, protected
  paths, folders untouched, collisions, no overwrite, batch continues after errors, undo exactness
  (content + date), undo skips changed / gone / taken files, older-batch undo, settings.

## Known gaps
- No settings screen yet: edit `%APPDATA%\file-organizer\settings.json` by hand.
- Online-only OneDrive files are skipped (not downloaded just to move them).
- Exe is ~5.5 MB (OpenGL GUI stack); no exe icon in Explorer, only the window icon.
- Moves across drives are copy-then-remove by Windows; if removal fails the file is reported, not lost.
- GUI itself is not covered by automated tests (only the library is).
