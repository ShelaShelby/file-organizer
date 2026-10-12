//! Sending files to the Windows Recycle Bin. Nothing here deletes a file
//! permanently: if Windows would delete instead of recycling, the operation
//! is cancelled and the file is left where it is.

use std::io;
use std::path::{Path, PathBuf};

/// Something that can send one file to the Recycle Bin. The app uses
/// [`RecycleBin`]; tests use a stand-in that moves files to a folder.
pub trait Recycler: Send + Sync {
    /// Sends `path` to the Recycle Bin. Returns where the file now is inside
    /// the Recycle Bin, if Windows tells us. On error the file must still be
    /// where it was.
    fn recycle(&self, path: &Path) -> io::Result<Option<PathBuf>>;
}

/// The real Windows Recycle Bin.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecycleBin;

impl Recycler for RecycleBin {
    fn recycle(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        if !path.is_absolute() {
            return Err(io::Error::other("path is not absolute"));
        }
        sys::recycle(path)
    }
}

#[cfg(windows)]
mod sys {
    //! Uses the documented shell API `IFileOperation` with
    //! `FOFX_RECYCLEONDELETE`. A progress sink checks every item in
    //! `PreDeleteItem`: if Windows did not set `TSF_DELETE_RECYCLE_IF_POSSIBLE`
    //! (no Recycle Bin on that drive, file too big, ...) the item would be
    //! deleted for good, so the sink returns `E_ABORT` and nothing happens.
    use std::cell::RefCell;
    use std::io;
    use std::path::{Path, PathBuf};
    use windows::core::{implement, ComObject, Ref, Result as WinResult, HRESULT, HSTRING, PCWSTR};
    use windows::Win32::Foundation::E_ABORT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
        SHCreateItemFromParsingName, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NO_UI,
        SIGDN_FILESYSPATH, TSF_DELETE_RECYCLE_IF_POSSIBLE,
    };

    #[implement(IFileOperationProgressSink)]
    struct Guard {
        refused: RefCell<bool>,
        new_place: RefCell<Option<PathBuf>>,
        result: RefCell<HRESULT>,
    }

    impl IFileOperationProgressSink_Impl for Guard_Impl {
        fn PreDeleteItem(&self, flags: u32, _item: Ref<IShellItem>) -> WinResult<()> {
            if flags & (TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32) == 0 {
                *self.refused.borrow_mut() = true;
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PostDeleteItem(&self, _f: u32, _i: Ref<IShellItem>, hr: HRESULT, created: Ref<IShellItem>) -> WinResult<()> {
            *self.result.borrow_mut() = hr;
            if let Some(item) = created.as_ref() {
                // SAFETY: documented call; the returned string is freed below.
                unsafe {
                    if let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                        if let Ok(s) = p.to_string() {
                            *self.new_place.borrow_mut() = Some(PathBuf::from(s));
                        }
                        CoTaskMemFree(Some(p.0 as *const _));
                    }
                }
            }
            Ok(())
        }
        fn StartOperations(&self) -> WinResult<()> {
            Ok(())
        }
        fn FinishOperations(&self, _hr: HRESULT) -> WinResult<()> {
            Ok(())
        }
        fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostRenameItem(
            &self,
            _: u32,
            _: Ref<IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<IShellItem>,
        ) -> WinResult<()> {
            Ok(())
        }
        fn PreMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostMoveItem(
            &self,
            _: u32,
            _: Ref<IShellItem>,
            _: Ref<IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<IShellItem>,
        ) -> WinResult<()> {
            Ok(())
        }
        fn PreCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostCopyItem(
            &self,
            _: u32,
            _: Ref<IShellItem>,
            _: Ref<IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<IShellItem>,
        ) -> WinResult<()> {
            Ok(())
        }
        fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostNewItem(
            &self,
            _: u32,
            _: Ref<IShellItem>,
            _: &PCWSTR,
            _: &PCWSTR,
            _: u32,
            _: HRESULT,
            _: Ref<IShellItem>,
        ) -> WinResult<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _: u32, _: u32) -> WinResult<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> WinResult<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> WinResult<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> WinResult<()> {
            Ok(())
        }
    }

    /// COM is set up for this thread for the duration of one call.
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: matches the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
    }

    pub fn recycle(path: &Path) -> io::Result<Option<PathBuf>> {
        // SAFETY: documented COM calls; all interfaces are released by Drop.
        unsafe {
            let _com = Com(CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok());
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(err)?;
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(err)?;
            op.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE).map_err(err)?;
            let g = ComObject::new(Guard {
                refused: RefCell::new(false),
                new_place: RefCell::new(None),
                result: RefCell::new(HRESULT(0)),
            });
            let sink: IFileOperationProgressSink = g.to_interface();
            let cookie = op.Advise(&sink).map_err(err)?;
            op.DeleteItem(&item, None).map_err(err)?;
            let performed = op.PerformOperations();
            let aborted = op.GetAnyOperationsAborted().map(|b| b.as_bool()).unwrap_or(true);
            let _ = op.Unadvise(cookie);
            if *g.refused.borrow() {
                return Err(io::Error::other(
                    "Windows cannot put this file in the Recycle Bin (it would be deleted for good), so it was left alone",
                ));
            }
            performed.map_err(err)?;
            let hr = *g.result.borrow();
            if aborted || hr.is_err() {
                return Err(io::Error::other(format!(
                    "Windows did not send the file to the Recycle Bin (0x{:08X})",
                    hr.0
                )));
            }
            if path.symlink_metadata().is_ok() {
                return Err(io::Error::other("the file is still there after sending it to the Recycle Bin"));
            }
            let place = g.new_place.borrow().clone();
            Ok(place)
        }
    }

    fn err(e: windows::core::Error) -> io::Error {
        io::Error::other(format!("{} (0x{:08X})", e.message(), e.code().0))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn recycle(_path: &Path) -> io::Result<Option<PathBuf>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "the Recycle Bin is only available on Windows"))
    }
}
