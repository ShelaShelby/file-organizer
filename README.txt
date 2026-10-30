File Organizer
==============

Tidies the loose files in your Downloads, Desktop and Documents folders,
finds duplicate files, lists your biggest and oldest files, cleans up messy
file names, and can sort finished downloads in the background.

How to use
1. Unzip and double-click file-organizer.exe (no install needed).
2. Press "Scan". You see a list of what WOULD be moved. Nothing has moved yet.
3. Untick any file you want to keep where it is.
4. Press "Confirm" to move the ticked files.
5. Changed your mind? Press "Undo last", or "Undo" next to any entry in History.

Tabs
- Sort: the steps above.
- Duplicates: press "Find duplicates" (all sub-folders are included; Cancel stops it).
  Each group holds files with exactly the same content. Nothing is ticked at
  first: tick a group (or "Select all") to clean it up. The oldest copy outside
  Downloads is kept; tick "Keep" on another copy to change that. For programs
  (.exe, .msi, ...) every copy is kept until you untick "Keep" yourself.
  "Confirm" shows the exact list of files that will go; they go to the Recycle Bin.
  Every group always keeps at least one copy.
- Big and old: lists the 200 biggest files (default 500 MB or more) and the 200
  oldest files (not changed for 365 days). List only. Per file: "Open folder", or
  "Recycle" (asks first). A third list, "Old installers", shows setup files
  (.exe, .msi, .msix, .appx, ...) not changed for 30 days, only at the top of
  Downloads and in Downloads\Installers\<month>. Nothing is ticked at first; tick
  what you want gone and press "Send ticked to the Recycle Bin" (asks first).
  Click the File / Size / Last changed headers to sort a list.
- Rename: press "Find messy names". Only three things are fixed:
  "photo (1) (2).jpg" -> "photo.jpg" (two or more counters; a single " (1)" stays),
  "Copy of report.docx" / "report - Copy.docx" -> "report.docx", and extra spaces
  ("my  file .txt" -> "my file.txt"). The extension never changes and the file
  stays in its folder. If the new name is taken, the file is left alone.
  Untick what you want to keep, press "Rename ticked". Undo works from History.
- Settings (top right): days for the Sort rule, big-file size, old-file days,
  old-installer days, smallest file size for duplicates, colours (same as
  Windows / dark / light), extra folders, "Start with Windows" (it shows which
  copy of the app it starts; "Use this copy" fixes it after you moved the app).

Extra folders
- In Settings press "Add folder…" to add up to 10 more folders (e.g. a Projects
  folder). They are handled like Documents (loose files untouched for 7 days go to
  <folder>\<Type>\<YYYY-MM>) and are included in Duplicates, Big and old and
  Rename. Background sorting stays Downloads only.
- Refused, with the reason: network or device paths, links/junctions, hidden or
  system folders, Windows / Program Files / AppData / your user folder itself /
  a whole drive, the app's own folders, and folders inside (or holding) another
  chosen folder.

Background sorting (Downloads only)
- Press "Start background sorting" (top right). A File Organizer icon appears in
  the tray (bottom right, maybe under the ^ arrow); no window stays open.
- Every 15 seconds it looks at the loose files in Downloads. A file is moved only
  when its download is clearly finished: not changed for 2 minutes, same size at
  two checks in a row, not in use, and not a .crdownload/.part/.partial/.tmp/
  .download file. Same folders as Sort: Downloads\<Type>\<YYYY-MM>.
- After a move a small note shows for 10 seconds with an "Undo" button.
- Right-click the tray icon: Open Organizer (brings the open window to the front), Pause (stays paused after a restart),
  Undo last auto-sort, Start with Windows, Quit.
- Start with Windows is off unless you turn it on (it only adds an entry for your
  own user account; no admin rights needed). Turning it off removes the entry.

Removed files
- Files are only ever sent to the Recycle Bin, never deleted for good. If Windows
  cannot put a file in the Recycle Bin, it is left alone and reported.
- History lists each removal with the old places of the files. To get a file back,
  open the Recycle Bin, right-click it and choose "Restore".

Where files go
- Downloads\photo.jpg   -> Downloads\Images\2026-10\photo.jpg
- Documents\notes.txt   -> Documents\Documents\2026-10\notes.txt
- Desktop\report.pdf    -> Documents\From Desktop\Documents\2026-10\report.pdf
  (the month is when the file was last changed)

Safety
- Nothing is ever deleted for good and no file is ever overwritten
  (if a name is taken, " (2)", " (3)" ... is added).
- Only loose files are moved. Your own folders and what is inside them are never touched.
- Program and project folders (node_modules, .git, a folder with package.json,
  *.sln, or an .exe next to a .dll, ...) and File Organizer itself are always left
  alone. See SECURITY.md for what the app protects against.
- Skipped: hidden/system files, shortcuts, desktop.ini, files still downloading
  or in use, and files changed recently (Desktop/Documents: 7 days, Downloads: 2 minutes).

Settings and history: %APPDATA%\file-organizer\
- settings.json  (days, sizes, and which extensions go in which folder)
- journal.jsonl  (every move, rename and file sent to the Recycle Bin;
  background moves are marked "auto")
