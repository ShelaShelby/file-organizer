File Organizer
==============

Tidies the loose files in your Downloads, Desktop and Documents folders,
finds duplicate files, and lists your biggest and oldest files.

How to use
1. Unzip and double-click file-organizer.exe (no install needed).
2. Press "Scan". You see a list of what WOULD be moved. Nothing has moved yet.
3. Untick any file you want to keep where it is.
4. Press "Confirm" to move the ticked files.
5. Changed your mind? Press "Undo last", or "Undo" next to any entry in History.

Tabs
- Sort: the steps above.
- Duplicates: press "Find duplicates" (all sub-folders are included; Cancel stops it).
  Each group holds files with exactly the same content. The copy with the oldest
  date is kept; tick "Keep" on another copy to change that, or untick a whole group.
  "Confirm" shows the exact list of files that will go; they go to the Recycle Bin.
  Every group always keeps at least one copy.
- Big and old: lists the 200 biggest files (default 500 MB or more) and the 200
  oldest files (not changed for 365 days). List only. Per file: "Open folder", or
  "Recycle" (asks first).
- Settings (top right): days for the Sort rule, big-file size, old-file days,
  smallest file size for duplicates.

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
- Skipped: hidden/system files, shortcuts, desktop.ini, files still downloading
  or in use, and files changed recently (Desktop/Documents: 7 days, Downloads: 2 minutes).

Settings and history: %APPDATA%\file-organizer\
- settings.json  (days, sizes, and which extensions go in which folder)
- journal.jsonl  (every move and every file sent to the Recycle Bin)
