//! Keep a relative path inside one checkout.
//!
//! `Path::join` drops the base when the joined path is absolute, so absolute
//! paths and `..` are rejected before any filesystem call. A symlink that
//! resolves outside the checkout is rejected after canonicalization.

use std::path::{Component, Path, PathBuf};

use super::ToolError;

pub(crate) fn resolve_inside(root: &Path, relative: &str) -> Result<PathBuf, ToolError> {
    if relative.is_empty() || relative.contains('\0') {
        return Err(ToolError::Escapes);
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(ToolError::Escapes);
    }
    for component in relative_path.components() {
        match component {
            Component::Normal(part) if part.to_str().is_some() => {}
            Component::CurDir => {}
            Component::Normal(_)
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(ToolError::Escapes),
        }
    }

    let root = root.canonicalize().map_err(|_| ToolError::NotFound)?;
    let joined = root.join(relative_path);
    let canonical = joined.canonicalize().map_err(|_| ToolError::NotFound)?;
    if !canonical.starts_with(&root) {
        return Err(ToolError::Escapes);
    }
    Ok(canonical)
}

pub(crate) fn relative_display(root: &Path, path: &Path) -> Result<String, ToolError> {
    let root = root.canonicalize().map_err(|_| ToolError::NotFound)?;
    let path = path.canonicalize().map_err(|_| ToolError::NotFound)?;
    if !path.starts_with(&root) {
        return Err(ToolError::Escapes);
    }
    Ok(path
        .strip_prefix(&root)
        .map_err(|_| ToolError::Escapes)?
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}
