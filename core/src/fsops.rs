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
        return Err(io::Error::other(format!(
            "copied to {}, but the original could not be removed, so both are there now (nothing is lost)",
            dst.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn move_no_replace(src: &Path, dst: &Path) -> io::Result<()> {
    // hard_link fails if dst exists, so this cannot overwrite.
    std::fs::hard_link(src, dst)?;
    std::fs::remove_file(src)
}

/// Identity of the file behind a path: (volume, file id). Two paths with the
/// same identity are the SAME file (a hard link, or one folder reached by two
/// paths), not two copies: removing one of them would remove the only copy.
#[cfg(windows)]
pub fn file_id(path: &Path) -> io::Result<(u64, u128)> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    // Attributes only, every kind of sharing allowed: never blocks another program.
    let f = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    // SAFETY: FILE_ID_INFO is plain data; the handle is valid while `f` lives
    // and the size passed is the size of the buffer.
    let info = unsafe {
        let mut info: FILE_ID_INFO = std::mem::zeroed();
        let ok = GetFileInformationByHandleEx(
            f.as_raw_handle() as _,
            FileIdInfo,
            &mut info as *mut FILE_ID_INFO as *mut _,
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        info
    };
    Ok((info.VolumeSerialNumber, u128::from_le_bytes(info.FileId.Identifier)))
}

#[cfg(not(windows))]
pub fn file_id(path: &Path) -> io::Result<(u64, u128)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path)?;
    Ok((m.dev(), u128::from(m.ino())))
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

/// True if any part of the path is `.` or `..`. Checked on the text, because
/// `Path::components` silently drops `.` parts.
pub fn has_dot_parts(p: &Path) -> bool {
    p.to_string_lossy().split(['\\', '/']).any(|part| part == "." || part == "..")
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
    move_into_tracked(src, dir, file_name).map(|(p, _)| p)
}

/// [`move_into`], also returning the folders this call had to create
/// (deepest first). Only these may be removed again on undo, and only if
/// they are empty then: a folder that was already there is never removed.
pub fn move_into_tracked(src: &Path, dir: &Path, file_name: &str) -> io::Result<(PathBuf, Vec<PathBuf>)> {
    let mut missing = Vec::new();
    let mut d = dir;
    while d.symlink_metadata().is_err() {
        missing.push(d.to_path_buf());
        match d.parent() {
            Some(p) if !p.as_os_str().is_empty() => d = p,
            _ => break,
        }
    }
    std::fs::create_dir_all(dir)?;
    for n in 1..10_000 {
        let dst = dir.join(numbered_name(file_name, n));
        if dst.symlink_metadata().is_ok() {
            continue;
        }
        match move_no_replace(src, &dst) {
            Ok(()) => return Ok((dst, missing)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                remove_empty_dirs(&missing);
                return Err(e);
            }
        }
    }
    remove_empty_dirs(&missing);
    Err(io::Error::other("no free file name found"))
}

/// Removes these folders (deepest first) while they are EMPTY; stops at the
/// first one that is not. `remove_dir` refuses a folder with anything in
/// it, and a link is never followed, so no file can be lost.
pub fn remove_empty_dirs(dirs: &[PathBuf]) {
    for d in dirs {
        let is_plain_dir = d.symlink_metadata().is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        if !is_plain_dir || std::fs::remove_dir(d).is_err() {
            return;
        }
    }
}
