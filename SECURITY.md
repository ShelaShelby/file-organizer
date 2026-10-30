# Security

## What the app can do
- Move loose files inside the folders you chose (Downloads, Desktop, Documents and up to 10 extra
  folders) into `<Type>\<YYYY-MM>` folders, rename junk names in place, and send files you ticked to
  the Recycle Bin. Every change is written to a journal and can be undone (Recycle Bin: restore there).
- It runs as you (manifest `asInvoker`), never asks for admin rights and never uses the network.

## What it never does
- Delete a file for good or replace an existing file (moves fail instead of overwriting). The only
  file it replaces is its own `settings.json`, atomically.
- Touch Windows, Program Files, ProgramData, AppData, the profile folder itself or drive roots. These
  places come from the Windows Known Folder API, not from environment variables, and each chosen folder
  is resolved to its final path first, so a junction cannot hide them.
- Enter links, junctions, hidden or system folders, program folders (an .exe next to a .dll) or project
  folders (.git, node_modules, package.json, Cargo.toml, *.sln, ...), or touch itself.

## What it protects against
- A hand-edited or damaged journal: Undo checks both paths (no `..`, inside a chosen folder, normal
  folders on the way, never app data) and skips anything that looks wrong.
- A tampered preview: targets are recomputed and compared before any move.
- Files changed after the scan: size, time, lock and (for duplicates) the full content hash are checked
  again right before each file is touched.
- DLLs planted next to the exe (e.g. in Downloads): the exe loads its own imports from System32 only,
  and Explorer is started by its full path.
- Removing the only copy by mistake: duplicates are compared by the real file behind each path
  (volume + file id), so a hard link or a folder reached twice is never treated as a second copy.
- Removing your folders on Undo: only folders the app created for a move (recorded in the journal)
  are removed, and only while empty.
- Two copies running at once (window and background sorter) share one lock, so no file moves twice.

## Known limits
- A program running as the same Windows user can change your files, the journal or settings.json
  itself; the app cannot stop that.
- The exe is not code-signed, so SmartScreen may warn the first time you run it.
- The graphics driver loads OpenGL DLLs dynamically at run time; that loading is not restricted.
- The Recycle Bin can be emptied by you, by Storage Sense or by Windows when it is full (oldest
  files first); after that a removed file is gone.
- Moving a file to another drive is copy-then-delete. If Windows cannot remove the original, both
  are left and the copy's place is reported; nothing is lost.

## Reporting a problem
Open an issue on GitHub. Please do not include personal files.
