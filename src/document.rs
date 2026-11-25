//! A text document and how it is read from and written to disk.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const BOM: &str = "\u{feff}";

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok()?,
        len: meta.len(),
    })
}

pub struct Document {
    /// Unique for the lifetime of the process.
    pub id: u64,
    /// Always uses `\n`; the original line ending is restored on save.
    pub text: String,
    pub path: Option<PathBuf>,
    revision: u64,
    saved_revision: u64,
    crlf: bool,
    bom: bool,
    disk: Option<Stamp>,
}

impl Document {
    pub fn untitled() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            text: String::new(),
            path: None,
            revision: 0,
            saved_revision: 0,
            crlf: false,
            bom: false,
            disk: None,
        }
    }

    /// Opens `path`. A path that does not exist yet gives an empty document
    /// that will be created on the first save.
    pub fn open(path: &Path) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let mut doc = Self::untitled();
        match fs::read_to_string(&path) {
            Ok(mut text) => {
                if let Some(rest) = text.strip_prefix(BOM) {
                    doc.bom = true;
                    text = rest.to_owned();
                }
                // Only normalize files that are consistently CRLF, so that
                // files with mixed endings are written back unchanged.
                let crlf = text.matches("\r\n").count();
                if crlf > 0 && crlf == text.matches('\n').count() {
                    doc.crlf = true;
                    text = text.replace("\r\n", "\n");
                }
                doc.text = text;
                doc.disk = stamp(&path);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        doc.path = Some(path);
        Ok(doc)
    }

    pub fn title(&self) -> String {
        self.path.as_deref().and_then(Path::file_name).map_or_else(
            || "Untitled".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_changed(&mut self) {
        self.revision += 1;
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// An untitled document that was never typed into.
    pub fn is_pristine(&self) -> bool {
        self.path.is_none() && self.revision == 0 && self.text.is_empty()
    }

    /// True when another program changed the file since we read or wrote it.
    pub fn changed_on_disk(&self) -> bool {
        match &self.path {
            Some(path) => stamp(path).is_some_and(|now| Some(now) != self.disk),
            None => false,
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::other("document has no file name"))?;
        self.write_to(&path)
    }

    pub fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        self.write_to(&path)?;
        self.path = Some(path);
        Ok(())
    }

    fn write_to(&mut self, path: &Path) -> io::Result<()> {
        let mut out = String::new();
        if self.bom {
            out.push_str(BOM);
        }
        if self.crlf {
            out.push_str(&self.text.replace('\n', "\r\n"));
        } else {
            out.push_str(&self.text);
        }
        write_atomic(path, out.as_bytes())?;
        self.disk = stamp(path);
        self.saved_revision = self.revision;
        Ok(())
    }
}

/// Writes through a temporary file so a failed save never truncates the original.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Write to the target of a symlink instead of replacing the link itself.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::other("not a file path"))?;
    let tmp = target.with_file_name(format!(
        ".{}.mde-{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        if let Ok(meta) = fs::metadata(&target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mde-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn round_trip(name: &str, original: &str) {
        let path = temp(name);
        fs::write(&path, original).unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.mark_changed();
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn keeps_line_endings_and_bom() {
        round_trip("lf.md", "a\nb\n");
        round_trip("crlf.md", "a\r\nb\r\n");
        round_trip("mixed.md", "a\r\nb\n");
        round_trip("bom.md", "\u{feff}# hi\r\n");
    }

    #[test]
    fn detects_outside_changes() {
        let path = temp("outside.md");
        fs::write(&path, "one").unwrap();
        let doc = Document::open(&path).unwrap();
        assert!(!doc.changed_on_disk());
        fs::write(&path, "something longer").unwrap();
        assert!(doc.changed_on_disk());
    }

    #[test]
    fn missing_file_is_created_on_save() {
        let path = temp("new.md");
        let _ = fs::remove_file(&path);
        let mut doc = Document::open(&path).unwrap();
        doc.text.push_str("hello");
        doc.mark_changed();
        assert!(doc.is_dirty());
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_keeps_the_link() {
        let target = temp("target.md");
        let link = temp("link.md");
        let _ = fs::remove_file(&link);
        fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut doc = Document::open(&link).unwrap();
        doc.text = "new".into();
        doc.mark_changed();
        doc.save().unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }
}
