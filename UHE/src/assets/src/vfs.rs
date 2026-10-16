use std::path::PathBuf;

use crate::error::{Error, Result};

/// The file system an asset path is resolved against.
///
/// The C++ `Vfs` mounted virtual prefixes to physical roots (`game:`,
/// `engine:`) and `FileSystem` bridged the legacy unqualified paths into it.
/// Both are one type here: an ordered list of roots, searched in order, first
/// match wins. Layered overrides (a mod directory mounted ahead of the game
/// directory) are a mount order, not a second mechanism.
///
/// Virtual paths use `/` on every platform, and a `\` is treated as a separator
/// too, so a path written on Windows still resolves on Linux. A path that would
/// leave its root (`../`, or an absolute/drive-qualified path) is refused
/// rather than resolved: `Vfs::ResolveAll` in the C++ compared the resolved
/// path against the mount root with `std::mismatch` and *skipped* the mount on
/// escape, which turns a traversal attempt into "not found" and hides it.
pub struct Vfs {
    roots: Vec<PathBuf>,
}

impl Vfs {
    pub fn new() -> Self {
        Self { roots: Vec::new() }
    }

    /// Appends a search root. Re-mounting the same directory is a no-op rather
    /// than a duplicate, because a root that appears twice changes nothing
    /// about resolution but does change `root_count`.
    pub fn mount(&mut self, dir: impl Into<PathBuf>) {
        let dir = dir.into();
        if !self.roots.iter().any(|root| root == &dir) {
            self.roots.push(dir);
        }
    }

    /// The search roots, in resolution order.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The first root that holds `virtual_path`, or `Error::NotFound`.
    pub fn resolve(&self, virtual_path: &str) -> Result<PathBuf> {
        let relative = relative_path(virtual_path)?;
        for root in &self.roots {
            let candidate = root.join(&relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(Error::NotFound(virtual_path.to_owned()))
    }

    /// Every root that holds `virtual_path`, in resolution order.
    pub fn resolve_all(&self, virtual_path: &str) -> Result<Vec<PathBuf>> {
        let relative = relative_path(virtual_path)?;
        Ok(self
            .roots
            .iter()
            .map(|root| root.join(&relative))
            .filter(|candidate| candidate.is_file())
            .collect())
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

/// Turns a virtual path into a root-relative one, or refuses it.
///
/// `.` and empty components are dropped, `..` is a refusal (not a pop: a
/// legitimate `..` would only ever leave the root, which is the case this
/// exists to catch), and a component ending in `:` is a Windows drive prefix,
/// which would silently become absolute under `Path::join`.
fn relative_path(virtual_path: &str) -> Result<PathBuf> {
    if virtual_path.is_empty() {
        return Err(Error::NotFound(String::new()));
    }

    let mut relative = PathBuf::new();
    for component in virtual_path.split(['/', '\\']) {
        match component {
            "" | "." => continue,
            ".." => return Err(Error::EscapesRoot(virtual_path.to_owned())),
            component => {
                if component.ends_with(':') {
                    return Err(Error::EscapesRoot(virtual_path.to_owned()));
                }
                relative.push(component);
            }
        }
    }

    if relative.as_os_str().is_empty() {
        return Err(Error::NotFound(virtual_path.to_owned()));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_strips_separators_and_dots() {
        assert_eq!(
            relative_path("a/b/c.png").unwrap(),
            PathBuf::from("a/b/c.png")
        );
        assert_eq!(relative_path("/a/b.png").unwrap(), PathBuf::from("a/b.png"));
        assert_eq!(
            relative_path("./a//b.png").unwrap(),
            PathBuf::from("a/b.png")
        );
        assert_eq!(relative_path("a\\b.png").unwrap(), PathBuf::from("a/b.png"));
    }

    #[test]
    fn relative_path_refuses_escapes() {
        assert!(matches!(
            relative_path("../secret"),
            Err(Error::EscapesRoot(_))
        ));
        assert!(matches!(
            relative_path("a/../../secret"),
            Err(Error::EscapesRoot(_))
        ));
        assert!(matches!(
            relative_path("C:/windows/system32"),
            Err(Error::EscapesRoot(_))
        ));
    }

    #[test]
    fn relative_path_rejects_what_names_nothing() {
        assert!(matches!(relative_path(""), Err(Error::NotFound(_))));
        assert!(matches!(relative_path("."), Err(Error::NotFound(_))));
    }
}
