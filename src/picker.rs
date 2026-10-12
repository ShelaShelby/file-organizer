//! The native Windows "choose a folder" dialog (IFileOpenDialog with
//! FOS_PICKFOLDERS). Only returns a path; every check is done in core.

use std::path::PathBuf;

#[cfg(windows)]
pub fn pick_folder() -> Option<PathBuf> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };
    // SAFETY: documented COM calls on this (UI) thread; interfaces are
    // released by Drop and the returned string is freed below.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = (|| {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let opts = dialog.GetOptions().ok()?;
            dialog.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
            dialog.Show(None).ok()?; // cancelled -> None
            let item = dialog.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = name.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(name.0 as *const _));
            path
        })();
        if com {
            CoUninitialize();
        }
        result
    }
}

#[cfg(not(windows))]
pub fn pick_folder() -> Option<PathBuf> {
    None
}
