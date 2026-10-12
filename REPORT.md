# Report – Stage 4
## Part 1 – security
- Undo checks both journal paths (no `..`/`.`, inside a current root, normal folders on the way, never
  protected or app data) and skips bad lines as "journal entry looks wrong". A torn journal line gets a
  line break before the next record. settings.json: temp file + `MoveFileExW(REPLACE_EXISTING)` under
  `settings.lock`; an unreadable file is still never replaced.
- Sort re-check recomputes `root\type\month`, refuses `..`/`.`, compares with `path_key`. Type folder
  names go through `is_valid_windows_name` (no CON/NUL/COM1…). Protected places come from
  `SHGetKnownFolderPath` (plus the C:\ list); roots are resolved with their final path first.
- Never touched: the running exe ("File Organizer itself"), program/project folders (names, markers,
  exe+dll). Duplicates are re-hashed (full BLAKE3, exclusive open) right before removal.
- `/DEPENDENTLOADFLAG:0x800`, Explorer by full path, manifest (asInvoker, Win10/11, PerMonitorV2);
  CI checks the manifest bytes and that `--tray` is alive after 3 s. Background sorter: metadata only,
  opens a file just before moving it, max 200 per tick. Settings shows the exe "Start with Windows" runs.
- CI: actions pinned to SHAs, build `contents: read`, separate `release` job (`contents: write`, main
  only) publishing the zip, `file-organizer.zip.sha256` and the hash in the notes. SECURITY.md added.

## Parts 2–5
- One main window (mutex + `FindWindowW`/`SW_RESTORE`/`SetForegroundWindow`); note popup position is a
  pure, tested function using the primary monitor's DPI and work area.
- Old installers: third list in "Big and old" (top of Downloads + `Downloads\Installers\YYYY-MM` only,
  `installer_days` default 30), tick boxes, one confirm + recycle path.
- Up to 10 extra folders (native folder picker), sorted like Documents and included in Duplicates,
  Big and old and Rename; every refusal rule checked on add, load and in each batch. `RootKind::Extra(n)`
  and `Roots::kinds()` replace `RootKind::ALL`.
- Theme (System/Dark/Light), Segoe UI fonts, plain-text buttons, tab counts, sortable File/Size/Last
  changed headers, Recycle Bin / tidied totals, per-folder summary; widths checked against 760x420.

## Numbers
- Tests: 172 on Windows in Actions, all passing (96 old + 76 new); one stage-3 test adjusted for item 11.
- Exe size: 6,106,624 bytes (stage 3: 6,037,504). zip SHA-256 is in the release notes.
- cargo audit 0.22.2 (1307 advisories, 324 crates): no vulnerabilities, no warnings.

## Not checked (no interactive desktop in CI)
- Real window, tray, note popup, folder picker, bring-to-front, theme, Segoe UI: logic tested, not seen on screen.

## Stage 4b
- History: text wraps on top, small left-aligned Undo below it; tested inside 220 and 280 px panels.
- Status bar: Info (normal text: background on, Scan cancelled) vs Error (red: could not…, not saved…).
- Recycle Bin restore hint shown once under the totals; tab counts were already shown only after a scan.
