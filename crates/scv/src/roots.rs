//! Read-root confinement and regular-file checks.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::ToolError;

/// Configured absolute read roots. All resolved targets must remain beneath one.
#[derive(Debug, Clone)]
pub struct ReadRoots {
    roots: Vec<PathBuf>,
}

impl ReadRoots {
    /// Canonicalize each root. Relative roots are resolved against `current_dir`.
    pub fn new(
        roots: impl IntoIterator<Item = PathBuf>,
        current_dir: &Path,
    ) -> Result<Self, String> {
        let mut canonical_roots = Vec::new();
        for root in roots {
            let absolute = if root.is_absolute() {
                root
            } else {
                current_dir.join(root)
            };
            let canonical = absolute.canonicalize().map_err(|error| {
                format!(
                    "failed to resolve read root {}: {error}",
                    absolute.display()
                )
            })?;
            if !canonical.is_dir() {
                return Err(format!(
                    "read root is not a directory: {}",
                    canonical.display()
                ));
            }
            canonical_roots.push(canonical);
        }
        if canonical_roots.is_empty() {
            let canonical = current_dir.canonicalize().map_err(|error| {
                format!(
                    "failed to resolve default read root {}: {error}",
                    current_dir.display()
                )
            })?;
            canonical_roots.push(canonical);
        }
        Ok(Self {
            roots: canonical_roots,
        })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Resolve `requested` to a regular file strictly below a configured root.
    ///
    /// Symlinks are followed; the final target must remain under a root.
    pub fn resolve_readable_file(&self, requested: &str) -> Result<PathBuf, ToolError> {
        let path = Path::new(requested);
        if !path.is_absolute() {
            return Err(ToolError::path_not_allowed(requested));
        }

        match path.canonicalize() {
            Ok(canonical) => self.accept_regular_file(canonical, requested),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Do not walk system prefix symlinks (e.g. macOS `/var` → `/private/var`)
                // as escapes. Classify missing paths by the longest existing ancestor.
                if self.missing_path_under_root(path) {
                    Err(ToolError::not_found(requested))
                } else {
                    Err(ToolError::path_not_allowed(requested))
                }
            }
            Err(error) => Err(map_io_error(error, requested)),
        }
    }

    fn accept_regular_file(
        &self,
        canonical: PathBuf,
        requested: &str,
    ) -> Result<PathBuf, ToolError> {
        if !self.is_under_any_root(&canonical) {
            return Err(ToolError::path_not_allowed(requested));
        }
        let metadata = fs::metadata(&canonical).map_err(|error| map_io_error(error, requested))?;
        if !metadata.is_file() {
            return Err(ToolError::not_regular_file(requested));
        }
        Ok(canonical)
    }

    fn is_under_any_root(&self, canonical: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| canonical == root.as_path() || canonical.starts_with(root))
    }

    /// A missing path is "under a root" when the longest existing ancestor
    /// canonicalizes under a root (so `/var` vs `/private/var` is handled).
    fn missing_path_under_root(&self, path: &Path) -> bool {
        let mut ancestor = path.to_path_buf();
        while ancestor.pop() {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if let Ok(canonical_ancestor) = ancestor.canonicalize() {
                if !self.is_under_any_root(&canonical_ancestor) {
                    return false;
                }
                // Remaining components after the ancestor must not climb out with `..`.
                if let Ok(suffix) = path.strip_prefix(&ancestor) {
                    let mut depth = 0i32;
                    for component in suffix.components() {
                        match component {
                            Component::CurDir => {}
                            Component::ParentDir => {
                                depth -= 1;
                                if depth < 0 {
                                    return false;
                                }
                            }
                            Component::Normal(_) => depth += 1,
                            Component::RootDir | Component::Prefix(_) => return false,
                        }
                    }
                    return true;
                }
                return false;
            }
        }
        false
    }
}

fn map_io_error(error: io::Error, requested: &str) -> ToolError {
    match error.kind() {
        io::ErrorKind::NotFound => ToolError::not_found(requested),
        io::ErrorKind::PermissionDenied => ToolError::read_failed(requested),
        _ => ToolError::read_failed(requested),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ToolErrorCode;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn rejects_relative_paths() {
        let directory = tempdir().expect("temp");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let error = roots
            .resolve_readable_file("relative.txt")
            .expect_err("relative");
        assert_eq!(error.code, ToolErrorCode::PathNotAllowed);
    }

    #[test]
    fn rejects_directory_as_not_regular_file() {
        let directory = tempdir().expect("temp");
        let nested = directory.path().join("subdir");
        fs::create_dir(&nested).expect("mkdir");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let error = roots
            .resolve_readable_file(nested.to_str().expect("utf8"))
            .expect_err("dir");
        assert_eq!(error.code, ToolErrorCode::NotRegularFile);
    }

    #[test]
    fn rejects_symlink_escape() {
        let directory = tempdir().expect("temp");
        let outside = tempdir().expect("outside");
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "nope").expect("write secret");

        let link = directory.path().join("escape");
        symlink(&secret, &link).expect("symlink");

        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let error = roots
            .resolve_readable_file(link.to_str().expect("utf8"))
            .expect_err("escape");
        assert_eq!(error.code, ToolErrorCode::PathNotAllowed);
    }

    #[test]
    fn accepts_file_under_root() {
        let directory = tempdir().expect("temp");
        let file = directory.path().join("ok.txt");
        fs::write(&file, "hello").expect("write");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let resolved = roots
            .resolve_readable_file(file.to_str().expect("utf8"))
            .expect("ok");
        assert_eq!(resolved, file.canonicalize().expect("canon"));
    }

    #[test]
    fn missing_file_under_root_is_not_found() {
        let directory = tempdir().expect("temp");
        let missing = directory.path().join("missing.txt");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let error = roots
            .resolve_readable_file(missing.to_str().expect("utf8"))
            .expect_err("missing");
        assert_eq!(error.code, ToolErrorCode::NotFound);
    }

    #[test]
    fn path_outside_root_is_not_allowed() {
        let directory = tempdir().expect("temp");
        let outside = tempdir().expect("outside");
        let file = outside.path().join("x.txt");
        fs::write(&file, "x").expect("write");
        let roots =
            ReadRoots::new([directory.path().to_path_buf()], directory.path()).expect("roots");
        let error = roots
            .resolve_readable_file(file.to_str().expect("utf8"))
            .expect_err("outside");
        assert_eq!(error.code, ToolErrorCode::PathNotAllowed);
    }
}
