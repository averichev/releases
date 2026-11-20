use cap_std::fs::Dir;
use std::io;

/// Synchronize the directory already held by the storage capability.
pub(super) fn sync_directory(directory: &Dir) -> io::Result<()> {
    // cap-std opens Linux directories with O_PATH. Duplicating that handle
    // preserves O_PATH, for which fsync returns EBADF. Open "." for reading
    // relative to the capability so we flush the same directory even if it
    // was renamed, without resolving its original ambient path again.
    #[cfg(target_os = "linux")]
    let file = directory.open(".")?.into_std();
    #[cfg(not(target_os = "linux"))]
    let file = directory.try_clone()?.into_std_file();

    file.sync_all()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use cap_std::ambient_authority;

    #[test]
    fn sync_uses_the_open_directory_after_its_path_is_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("objects");
        std::fs::create_dir(&path).unwrap();
        let directory = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        std::fs::rename(&path, temporary.path().join("moved")).unwrap();
        std::fs::write(&path, b"original path is no longer a directory").unwrap();

        sync_directory(&directory).unwrap();
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"original path is no longer a directory"
        );
    }
}
