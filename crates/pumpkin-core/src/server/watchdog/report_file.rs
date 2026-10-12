use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
};

fn open_at(directory: &File, name: &CStr, flags: libc::c_int) -> io::Result<File> {
    // SAFETY: the borrowed descriptor and NUL-terminated name stay valid for this call.
    // A mode is supplied for O_CREAT; openat ignores it for directory-only opens.
    let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat returned a new descriptor exclusively owned here.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn secure_directory(directory: &File, uid: libc::uid_t) -> io::Result<()> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "watchdog private directory is not owned by this user",
        ));
    }
    directory.set_permissions(std::fs::Permissions::from_mode(0o700))
}

pub(super) fn open_report(directory: &Path, path: &Path) -> io::Result<(File, PathBuf)> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?;
    // SAFETY: geteuid only reads the effective identity of this process.
    let uid = unsafe { libc::geteuid() };
    let (private, published_directory) = if root.metadata()?.uid() == uid {
        secure_directory(&root, uid)?;
        (root, directory.to_path_buf())
    } else {
        // A shared root can be writable without granting permission to chmod it.
        // Bind creation and opens to descriptors, even if its entries are renamed.
        let name = CString::new(format!("watchdog-{uid}")).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "watchdog private directory name is invalid",
            )
        })?;
        // SAFETY: root owns a live directory descriptor and name is NUL-terminated.
        let created = unsafe { libc::mkdirat(root.as_raw_fd(), name.as_ptr(), 0o700) };
        if created < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let private = open_at(
            &root,
            &name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        secure_directory(&private, uid)?;
        (private, directory.join(format!("watchdog-{uid}")))
    };
    let filename = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "watchdog report filename is missing",
        )
    })?;
    let name = CString::new(filename.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "watchdog report filename contains NUL",
        )
    })?;
    let file = open_at(
        &private,
        &name,
        libc::O_WRONLY
            | libc::O_APPEND
            | libc::O_CREAT
            | libc::O_EXCL
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK,
    )?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "watchdog report is not an owned regular file",
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok((file, published_directory.join(filename)))
}
