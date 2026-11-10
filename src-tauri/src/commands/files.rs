//! Native file choosing for CSV import, profile export/restore, and artwork.
//!
//! Desktop dialogs return real paths and those are used directly. Android's document picker
//! returns `content://` URIs instead, which `std::fs` cannot open. There the file is copied
//! through the content resolver to a private temp file (reads) or written to a temp file and
//! then copied out to the chosen document (saves), so the callers only ever deal with paths.
//! The renderer never sees a path or a URI.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::domain::ids::new_id;
use crate::error::{AppError, AppResult};

/// Private scratch folder (under the app cache dir) for files staged from content URIs.
const STAGE_DIR: &str = "picked";

/// A file chosen for reading. On Android the content is a private temp copy that is deleted
/// when this is dropped.
#[derive(Debug)]
pub struct PickedFile {
    path: PathBuf,
    name: String,
    temp: bool,
}

impl PickedFile {
    fn real(path: PathBuf, fallback_name: &str) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| fallback_name.to_owned());
        Self {
            path,
            name,
            temp: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The chosen file's name, for display.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for PickedFile {
    fn drop(&mut self) {
        if self.temp {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Where to save. Write to [`SaveTarget::path`], then call [`SaveTarget::finish`].
pub struct SaveTarget {
    path: PathBuf,
    name: String,
    /// Android: the document to copy the finished temp file into.
    dest: Option<(AppHandle, FilePath)>,
}

impl SaveTarget {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved file's name, for display.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Complete the save. On desktop the file is already in place; on Android this copies the
    /// temp file into the chosen document (replacing its contents).
    pub fn finish(mut self) -> AppResult<()> {
        let Some((app, dest)) = self.dest.take() else {
            return Ok(());
        };
        let result = (|| -> AppResult<()> {
            let mut src = std::fs::File::open(&self.path).map_err(save_error)?;
            let mut out = open_content(&app, dest, true).map_err(save_error)?;
            std::io::copy(&mut src, &mut out).map_err(save_error)?;
            out.flush().map_err(save_error)?;
            out.sync_all().map_err(save_error)
        })();
        let _ = std::fs::remove_file(&self.path);
        result
    }
}

impl Drop for SaveTarget {
    fn drop(&mut self) {
        // Only a temp file (Android) is ours to remove; a desktop path is the user's file.
        if self.dest.is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn save_error(e: std::io::Error) -> AppError {
    AppError::validation("file", format!("couldn't save the file: {e}"))
}

/// Copy at most `limit` bytes; a longer source is refused rather than truncated.
pub(crate) fn copy_limited(
    src: &mut impl Read,
    dst: &mut impl Write,
    limit: u64,
) -> AppResult<u64> {
    let copied = std::io::copy(&mut src.take(limit.saturating_add(1)), dst)
        .map_err(|e| AppError::validation("file", format!("couldn't read it: {e}")))?;
    if copied > limit {
        return Err(AppError::validation(
            "file",
            format!("larger than {} MB", limit / (1024 * 1024)),
        ));
    }
    Ok(copied)
}

/// A readable name from a content URI's last segment (`…/primary%3ADownload%2Fx.csv` →
/// `x.csv`), or `fallback` when the provider hides it behind an opaque id.
pub(crate) fn name_from_uri(uri: &str, fallback: &str) -> String {
    let segment = uri
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .unwrap_or("");
    let decoded = percent_decode(segment);
    let tail = decoded.rsplit(['/', ':']).next().unwrap_or("");
    let ok = tail.contains('.')
        && !tail.starts_with('.')
        && tail.len() <= 120
        && tail.chars().all(|c| !c.is_control() && c != '\\');
    if ok {
        tail.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn stage_path(app: &AppHandle, extension: &str) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::StorageUnavailable(format!("no cache dir: {e}")))?
        .join(STAGE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
    Ok(dir.join(format!("{}.{extension}", new_id())))
}

/// Forget staged files left by an earlier run (called at startup).
pub fn clear_staged(app: &AppHandle) {
    if let Ok(dir) = app.path().app_cache_dir() {
        let _ = std::fs::remove_dir_all(dir.join(STAGE_DIR));
    }
}

/// Open a picked document. `FilePath::Path` is a plain file; `FilePath::Url` only occurs on
/// Android and goes through the content resolver.
fn open_content(app: &AppHandle, file: FilePath, write: bool) -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_fs::{FsExt, OpenOptions};
        let mut opts = OpenOptions::new();
        if write {
            opts.write(true).truncate(true);
        } else {
            opts.read(true);
        }
        app.fs().open(file, opts)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let path = file
            .into_path()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        if write {
            std::fs::File::create(path)
        } else {
            std::fs::File::open(path)
        }
    }
}

/// Choose a file to read. `kind` names the dialog for end-to-end tests. Files larger than
/// `max_bytes` are refused. `None` when cancelled.
pub async fn pick_open(
    app: &AppHandle,
    kind: &str,
    title: &str,
    filter: (&str, &[&str]),
    max_bytes: u64,
    fallback_name: &str,
) -> AppResult<Option<PickedFile>> {
    #[cfg(feature = "e2e")]
    if let Some(p) = crate::e2e::dialog_path(kind) {
        return Ok(Some(PickedFile::real(p, fallback_name)));
    }
    #[cfg(not(feature = "e2e"))]
    let _ = kind;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(title)
        .add_filter(filter.0, filter.1)
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let chosen = rx
        .await
        .map_err(|_| AppError::Internal("file dialog closed unexpectedly".into()))?;
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    match chosen {
        FilePath::Path(path) => Ok(Some(PickedFile::real(path, fallback_name))),
        FilePath::Url(url) => {
            let name = name_from_uri(url.as_str(), fallback_name);
            let extension = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
                .unwrap_or("bin")
                .to_owned();
            let path = stage_path(app, &extension)?;
            // From here the temp file is owned (and cleaned up) by `staged`.
            let staged = PickedFile {
                path,
                name,
                temp: true,
            };
            let mut src = open_content(app, FilePath::Url(url), false)
                .map_err(|e| AppError::validation("file", format!("couldn't read it: {e}")))?;
            let mut dst = std::fs::File::create(staged.path())
                .map_err(|e| AppError::StorageUnavailable(e.to_string()))?;
            copy_limited(&mut src, &mut dst, max_bytes)?;
            Ok(Some(staged))
        }
    }
}

/// Choose where to save. `None` when cancelled.
pub async fn pick_save(
    app: &AppHandle,
    kind: &str,
    default_name: &str,
    filter: (&str, &[&str]),
) -> AppResult<Option<SaveTarget>> {
    #[cfg(feature = "e2e")]
    if let Some(p) = crate::e2e::dialog_path(kind) {
        return Ok(Some(SaveTarget {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| default_name.to_owned()),
            path: p,
            dest: None,
        }));
    }
    #[cfg(not(feature = "e2e"))]
    let _ = kind;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(filter.0, filter.1)
        .set_file_name(default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let chosen = rx
        .await
        .map_err(|_| AppError::Internal("file dialog closed unexpectedly".into()))?;
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    match chosen {
        FilePath::Path(path) => Ok(Some(SaveTarget {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| default_name.to_owned()),
            path,
            dest: None,
        })),
        FilePath::Url(url) => {
            let name = name_from_uri(url.as_str(), default_name);
            let extension = Path::new(default_name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bin");
            Ok(Some(SaveTarget {
                path: stage_path(app, extension)?,
                name,
                dest: Some((app.clone(), FilePath::Url(url))),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_up_to_the_limit_and_refuses_more() {
        let mut out = Vec::new();
        assert_eq!(copy_limited(&mut &b"abcd"[..], &mut out, 4).unwrap(), 4);
        assert_eq!(out, b"abcd");
        let err = copy_limited(&mut &b"abcde"[..], &mut Vec::new(), 4).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn names_come_from_the_uri_or_fall_back() {
        let n = |u: &str| name_from_uri(u, "import.csv");
        assert_eq!(
            n(
                "content://com.android.externalstorage.documents/document/primary%3ADownload%2Fmy%20albums.csv"
            ),
            "my albums.csv"
        );
        assert_eq!(n("content://media/external/file/1234"), "import.csv");
        assert_eq!(n("content://x.documents/document/msf%3A42"), "import.csv");
        assert_eq!(n("content://x/doc/a.csv?name=z#frag"), "a.csv");
        assert_eq!(n("content://x/doc/%2e%2e%2f"), "import.csv");
    }
}
