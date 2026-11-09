use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub is_dir: bool,
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedText {
    pub text: String,
    pub lossy: bool,
    pub stamp: FileStamp,
}

#[derive(Debug)]
pub enum PersistError {
    Io(io::Error),
    Conflict(PathBuf),
    Lossy(PathBuf),
    IsDirectory,
    TooLarge { limit: u64 },
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Io(error) => write!(f, "{error}"),
            PersistError::Conflict(path) => write!(
                f,
                "{} changed on disk since it was opened. Save refused and the file on disk was left as it is.",
                path.display()
            ),
            PersistError::Lossy(path) => write!(
                f,
                "{} was opened with lossy UTF-8. Saving would replace bytes that did not decode, so the save was refused.",
                path.display()
            ),
            PersistError::IsDirectory => write!(f, "That path is a folder"),
            PersistError::TooLarge { limit } => {
                let limit = *limit;
                let mib = limit / (1024 * 1024);
                if mib > 0 && limit == mib * 1024 * 1024 {
                    write!(f, "File is larger than {mib} MiB")
                } else {
                    write!(f, "File is larger than {limit} bytes")
                }
            }
        }
    }
}

impl std::error::Error for PersistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PersistError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PersistError {
    fn from(error: io::Error) -> Self {
        PersistError::Io(error)
    }
}

pub fn file_stamp(path: &Path) -> io::Result<FileStamp> {
    match fs::metadata(path) {
        Ok(meta) => Ok(FileStamp {
            modified: meta.modified().ok(),
            len: meta.len(),
            is_dir: meta.is_dir(),
            exists: true,
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileStamp {
            modified: None,
            len: 0,
            is_dir: false,
            exists: false,
        }),
        Err(error) => Err(error),
    }
}

pub fn read_text_file(path: &Path, max_bytes: u64) -> Result<OpenedText, PersistError> {
    const ATTEMPTS: usize = 4;
    for _ in 0..ATTEMPTS {
        let before = file_stamp(path)?;
        if !before.exists {
            return Err(PersistError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("could not open {}", path.display()),
            )));
        }
        if before.is_dir {
            return Err(PersistError::IsDirectory);
        }
        if before.len > max_bytes {
            return Err(PersistError::TooLarge { limit: max_bytes });
        }
        let bytes = fs::read(path)?;
        if bytes.len() as u64 > max_bytes {
            return Err(PersistError::TooLarge { limit: max_bytes });
        }
        let after = file_stamp(path)?;
        if before == after && after.len == bytes.len() as u64 {
            let lossy = std::str::from_utf8(&bytes).is_err();
            return Ok(OpenedText {
                text: String::from_utf8_lossy(&bytes).into_owned(),
                lossy,
                stamp: after,
            });
        }
    }
    Err(PersistError::Io(io::Error::other(format!(
        "{} changed while it was being opened",
        path.display()
    ))))
}

pub fn save_text(
    path: &Path,
    text: &str,
    baseline: Option<&FileStamp>,
    lossy: bool,
    acknowledge_lossy: bool,
) -> Result<FileStamp, PersistError> {
    if let Some(baseline) = baseline {
        let current = file_stamp(path)?;
        if current != *baseline {
            // mtime or length differs from open, so the disk file wins.
            return Err(PersistError::Conflict(path.to_path_buf()));
        }
    }
    if lossy && !acknowledge_lossy {
        // The editor text replaced undecodable bytes, so writing it would destroy the file.
        return Err(PersistError::Lossy(path.to_path_buf()));
    }
    replace_file(path, text.as_bytes())?;
    let stamp = file_stamp(path)?;
    if !stamp.exists || stamp.is_dir {
        return Err(PersistError::Io(io::Error::other(format!(
            "saved {} but it is not a file afterward",
            path.display()
        ))));
    }
    Ok(stamp)
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dest = replace_destination(path);
    let dir = parent_dir(&dest);
    let name = dest.file_name().unwrap_or(std::ffi::OsStr::new("buffer"));
    let (mut file, tmp) = match create_temp(dir, name) {
        Ok(created) => created,
        Err(error) => {
            return Err(unchanged(
                error.kind(),
                format!(
                    "could not create a temporary file in {}: {error}",
                    dir.display()
                ),
            ));
        }
    };
    if let Err(error) = write_synced(&mut file, bytes) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(unchanged(
            error.kind(),
            format!("could not write {}: {error}", tmp.display()),
        ));
    }
    if let Ok(meta) = fs::metadata(&dest)
        && meta.is_file()
    {
        let _ = fs::set_permissions(&tmp, meta.permissions());
        let _ = file.sync_all();
    }
    drop(file);
    // Rename is the replace, so a failure leaves the destination untouched.
    if let Err(error) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(unchanged(
            error.kind(),
            format!("could not replace {}: {error}", dest.display()),
        ));
    }
    // Directory fsync is the strongest durability std::fs can give the rename.
    if let Ok(dir_file) = File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

fn unchanged(kind: io::ErrorKind, message: String) -> io::Error {
    io::Error::new(
        kind,
        format!("{message}. The destination was left unchanged"),
    )
}

fn write_synced(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn replace_destination(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Follow the link so rename replaces the target file, not the link itself.
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn create_temp(dir: &Path, file_name: &std::ffi::OsStr) -> io::Result<(File, PathBuf)> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut last = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut name = std::ffi::OsString::from(".");
        name.push(file_name);
        name.push(".");
        name.push(std::process::id().to_string());
        name.push(".");
        name.push(n.to_string());
        name.push(".tmp");
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("could not create a temporary file")))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "hyperion-ide-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn assert_no_temps(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            assert!(!name.ends_with(".tmp"), "left temporary file {name}");
        }
    }

    #[test]
    fn save_replaces_the_file_with_the_full_new_contents() {
        let dir = Scratch::new("replace");
        let path = dir.path().join("note.txt");
        fs::write(&path, b"abcdefghij").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o640);
            fs::set_permissions(&path, permissions).unwrap();
        }
        let opened = read_text_file(&path, 1024).unwrap();
        let stamp = save_text(&path, "xy", Some(&opened.stamp), false, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"xy");
        assert_eq!(file_stamp(&path).unwrap(), stamp);
        assert_no_temps(dir.path());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        save_text(&path, "xy\n", Some(&stamp), false, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"xy\n");
    }

    #[test]
    fn failed_replace_leaves_the_original_in_place() {
        let dir = Scratch::new("keep");
        let dest = dir.path().join("folder");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("keep.txt"), b"keep-me").unwrap();
        let error = save_text(&dest, "new", None, false, false).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("left unchanged"),
            "error should say the destination was left unchanged: {message}"
        );
        assert_eq!(fs::read(dest.join("keep.txt")).unwrap(), b"keep-me");
        assert!(dest.is_dir());
        assert_no_temps(dir.path());
    }

    #[test]
    fn external_modification_is_not_overwritten() {
        let dir = Scratch::new("conflict");
        let path = dir.path().join("note.txt");
        fs::write(&path, b"original-long").unwrap();
        let opened = read_text_file(&path, 1024).unwrap();
        fs::write(&path, b"changed").unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH).unwrap();
        drop(file);
        let error = save_text(&path, "x", Some(&opened.stamp), false, false).unwrap_err();
        assert!(matches!(error, PersistError::Conflict(_)), "{error}");
        assert!(error.to_string().contains("left as it is"), "{}", error);
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        assert_no_temps(dir.path());
    }

    #[test]
    fn lossy_utf8_is_not_overwritten_without_acknowledge() {
        let dir = Scratch::new("lossy");
        let path = dir.path().join("bin.dat");
        fs::write(&path, b"a\xffb").unwrap();
        let opened = read_text_file(&path, 1024).unwrap();
        assert!(opened.lossy);
        assert_eq!(opened.text, "a\u{FFFD}b");
        let refused = save_text(&path, &opened.text, Some(&opened.stamp), true, false).unwrap_err();
        assert!(matches!(refused, PersistError::Lossy(_)), "{refused}");
        assert!(refused.to_string().contains("lossy UTF-8"), "{}", refused);
        assert_eq!(fs::read(&path).unwrap(), b"a\xffb");
        let stamp = save_text(&path, &opened.text, Some(&opened.stamp), true, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), "a\u{FFFD}b".as_bytes());
        let reread = read_text_file(&path, 1024).unwrap();
        assert!(!reread.lossy);
        assert_eq!(reread.stamp, stamp);
    }

    #[test]
    fn acknowledged_lossy_save_still_refuses_an_external_change() {
        let dir = Scratch::new("both");
        let path = dir.path().join("bin.dat");
        fs::write(&path, b"a\xffb").unwrap();
        let opened = read_text_file(&path, 1024).unwrap();
        fs::write(&path, b"zz\xff").unwrap();
        let error = save_text(&path, &opened.text, Some(&opened.stamp), true, true).unwrap_err();
        assert!(matches!(error, PersistError::Conflict(_)), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"zz\xff");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_save_replaces_the_target_not_the_link() {
        let dir = Scratch::new("link");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, b"old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let opened = read_text_file(&link, 1024).unwrap();
        save_text(&link, "new", Some(&opened.stamp), false, false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_no_temps(dir.path());
    }
}
