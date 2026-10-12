//! Low-level file operations. Nothing here can overwrite or delete a file.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// Moves `src` to `dst`. Fails with `AlreadyExists` if `dst` exists; never
/// replaces anything.
#[cfg(windows)]
pub fn move_no_replace(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH};
    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }
    let (s, d) = (wide(src), wide(dst));
    // No MOVEFILE_REPLACE_EXISTING: Windows refuses if the target exists.
    // SAFETY: both strings are NUL-terminated and live for the call.
    let ok = unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // Across drives Windows copies then deletes; if the delete failed the
    // original is still there. Report it (nothing is lost).
    if src.symlink_metadata().is_ok() {
        return Err(io::Error::other("copied, but the original could not be removed"));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn move_no_replace(src: &Path, dst: &Path) -> io::Result<()> {
    // hard_link fails if dst exists, so this cannot overwrite.
    std::fs::hard_link(src, dst)?;
    std::fs::remove_file(src)
}

/// "name.ext" -> "name (n).ext" (n >= 2).
pub fn numbered_name(file_name: &str, n: u32) -> String {
    if n < 2 {
        return file_name.to_string();
    }
    let p = Path::new(file_name);
    match (p.file_stem(), p.extension()) {
        (Some(stem), Some(ext)) if !stem.is_empty() => {
            format!("{} ({}).{}", stem.to_string_lossy(), n, ext.to_string_lossy())
        }
        _ => format!("{file_name} ({n})"),
    }
}

/// Case-insensitive key for a path (Windows file names ignore case).
pub fn path_key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// First free path in `dir` for `file_name`: not on disk and not in `taken`.
pub fn free_target(dir: &Path, file_name: &str, taken: &HashSet<String>) -> PathBuf {
    let mut n = 1;
    loop {
        let cand = dir.join(numbered_name(file_name, n));
        if cand.symlink_metadata().is_err() && !taken.contains(&path_key(&cand)) {
            return cand;
        }
        n += 1;
    }
}

/// Moves `src` into `dir` under `file_name`, or "file_name (2)" etc. if
/// that name is taken. Returns the final path.
pub fn move_into(src: &Path, dir: &Path, file_name: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 1..10_000 {
        let dst = dir.join(numbered_name(file_name, n));
        if dst.symlink_metadata().is_ok() {
            continue;
        }
        match move_no_replace(src, &dst) {
            Ok(()) => return Ok(dst),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("no free file name found"))
}
