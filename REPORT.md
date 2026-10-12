# Report – Stage 3

## Built
- `core/rename` (no GUI): walks the 3 roots with sub-folders (stage 2 walker and skip rules) and proposes
  renames only for: 2+ stacked counters (`photo (1) (2).jpg` → `photo.jpg`), `Copy of ` prefix / ` - Copy`
  suffix (any case), repeated / surrounding spaces. Extension never changes, same folder only, valid Windows
  name (no reserved/forbidden/empty names), no numbers invented. Skipped if the target exists (ignoring
  case) or two proposals share a target. Right before each rename: inside a root, unchanged size and
  last-changed time, not locked, target still free; then the stage 1 no-replace move. Journaled (`rename`).
- GUI tab "Rename": preview (old → new, folder), all ticked, "Rename ticked". Undo via History / Undo last:
  only if the file still has the new name, unchanged, and the old name is free.
- `core/autosort` + `--tray`: separate process, no window and no egui; raw Win32 tray icon, hidden window,
  15 s `SetTimer`, sleeps in `GetMessageW` (no new crates). Single instance via named mutex. Top-level
  Downloads only, reusing the stage 1 scan/execute (same rules, `Type\YYYY-MM`, journal, tag `auto`).
  Eligible = stage 1 checks (2 min, temp names, exclusive open) + changed > 60 s ago + same size and time at
  two checks + not `.crdownload/.part/.partial/.tmp/.download`.
- After a batch the tray starts `file-organizer.exe --note <batch>`: small always-on-top popup at the
  bottom right, "Moved N files to Downloads\<Type>\<YYYY-MM>", Undo button, closes after ~10 s.
- Tray menu: Open Organizer, Pause (checked; saved as `background_paused` in settings.json), Undo last
  auto-sort, Start with Windows (checkbox), Quit. Main app: "Start background sorting" button, Settings
  checkbox "Start with Windows" (HKCU `...\Run`, value `FileOrganizer` = `"<exe>" --tray`, default off,
  unticking removes it). Settings saves from the window keep the tray's pause switch.
- Two processes: every batch (sort, auto-sort, rename, recycle, undo) holds an exclusive lock on
  `journal.lock` (std `File::lock`) while it re-checks, moves and journals. The second waits and then
  finds the file gone, so nothing is moved twice and journal lines never interleave.
- Icon: `tools/make_icon.py` (pure Python) draws a folder with a green check badge → `assets/icon.ico`
  (16–64 px bitmaps + 256 px PNG, 38 KB). Embedded with `build.rs` + `embed-resource` (build-only
  dependency); used for exe, window (read from the .ico) and tray. Main window History marks
  "renamed" and "background" batches and refreshes when the tray writes the journal.

## Tests
- 97 tests on Windows in Actions: 35 stage 1 + 30 stage 2 + 31 stage 3 + 1 icon. Stage 3: each rename
  pattern, single counter left, extension kept, invalid/reserved/empty names, target exists (case-
  insensitive), same-target proposals, file created/changed/gone/locked after scan, tampered proposals,
  hidden/temp/shortcut/recent skipped, Undo + Undo refused; auto-sort: two checks needed, too new, size
  changing, temp extensions, locked, paused (and saved), top level only, undo last auto-sort only; journal
  lock exclusive, parallel batches move each file once; Run value written, read and removed in HKCU.
- Exe size: about 6 MB (exact number printed by the CI "Package" step).

## Known gaps
- The tray icon, its menu and the `--note` popup were never seen on a real desktop: CI has no
  interactive session, so they are compile-checked only. Popup position assumes system DPI.
- "Open Organizer" opens a new window even if one is already open. Tray Undo shows its result as a note.
- Rename Undo checks size + last-changed time, not file content.
