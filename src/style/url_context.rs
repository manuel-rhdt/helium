//! Where the relative URLs of the style sheet that is being parsed are read from.
//!
//! A relative URL in a style sheet is relative to the style sheet, not to the document. The
//! parser of a value has no way to be told where the style sheet is, so the callers that read a
//! style sheet say so for the time of the parsing.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The directories that the URLs of a style sheet depend on.
#[derive(Debug, Clone, Default)]
pub struct UrlContext {
    /// The directory of the style sheet: what relative URLs are relative to.
    pub sheet_dir: Option<PathBuf>,
    /// The directory of the document: the files must be inside of it (see `image::load_relative`).
    pub document_dir: Option<PathBuf>,
}

thread_local! {
    static CONTEXT: RefCell<UrlContext> = RefCell::new(UrlContext::default());
}

/// Runs `parse` with the style sheet in `sheet_dir` of the document in `document_dir`.
pub fn with<R>(sheet_dir: &Path, document_dir: &Path, parse: impl FnOnce() -> R) -> R {
    let context = UrlContext {
        sheet_dir: Some(sheet_dir.to_path_buf()),
        document_dir: Some(document_dir.to_path_buf()),
    };
    let previous = CONTEXT.with(|current| current.replace(context));
    let result = parse();
    CONTEXT.with(|current| *current.borrow_mut() = previous);
    result
}

/// The context of the style sheet that is being parsed.
pub fn current() -> UrlContext {
    CONTEXT.with(|current| current.borrow().clone())
}
