use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn aliases_input(output: &Path, input: &Path) -> io::Result<bool> {
    let output = match fs::canonicalize(output) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let input = fs::canonicalize(input)?;
    if output == input {
        return Ok(true);
    }
    same_file(&output, &input)
}

#[cfg(unix)]
fn same_file(output: &Path, input: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let output = fs::metadata(output)?;
    let input = fs::metadata(input)?;
    Ok(output.dev() == input.dev() && output.ino() == input.ino())
}

#[cfg(windows)]
fn same_file(output: &Path, input: &Path) -> io::Result<bool> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    // FILE_ID_INFO identifies hard links on the same volume. The standard
    // library's equivalent MetadataExt accessors are still unstable.
    #[repr(C)]
    #[derive(Default, PartialEq, Eq)]
    struct FileIdInfo {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            file: *mut c_void,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }

    fn identity(path: &Path) -> io::Result<FileIdInfo> {
        let file = fs::File::open(path)?;
        let mut info = FileIdInfo::default();
        // SAFETY: the handle remains open and the initialized output buffer has
        // the FILE_ID_INFO layout/size required by FileIdInfo (class 18).
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                18,
                (&mut info as *mut FileIdInfo).cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    Ok(identity(output)? == identity(input)?)
}

#[cfg(not(any(unix, windows)))]
fn same_file(_output: &Path, _input: &Path) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "comparison output file identity checks require Unix or Windows",
    ))
}

pub(super) fn resolve_input_path(project_root: &Path, raw_path: Option<&str>) -> Option<PathBuf> {
    let raw_path = raw_path?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    })
}
