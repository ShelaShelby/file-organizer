File Organizer
==============

Tidies the loose files in your Downloads, Desktop and Documents folders.

How to use
1. Unzip and double-click file-organizer.exe (no install needed).
2. Press "Scan". You see a list of what WOULD be moved. Nothing has moved yet.
3. Untick any file you want to keep where it is.
4. Press "Confirm" to move the ticked files.
5. Changed your mind? Press "Undo last", or "Undo" next to any entry in History.

Where files go
- Downloads\photo.jpg   -> Downloads\Images\2026-10\photo.jpg
- Documents\notes.txt   -> Documents\Documents\2026-10\notes.txt
- Desktop\report.pdf    -> Documents\From Desktop\Documents\2026-10\report.pdf
  (the month is when the file was last changed)

Safety
- Nothing is ever deleted and no file is ever overwritten
  (if a name is taken, " (2)", " (3)" ... is added).
- Only loose files are moved. Your own folders and what is inside them are never touched.
- Skipped: hidden/system files, shortcuts, desktop.ini, files still downloading
  or in use, and files changed recently (Desktop/Documents: 7 days, Downloads: 2 minutes).

Settings and history: %APPDATA%\file-organizer\
- settings.json  (number of days, and which extensions go in which folder)
- journal.jsonl  (every move, used for Undo)
