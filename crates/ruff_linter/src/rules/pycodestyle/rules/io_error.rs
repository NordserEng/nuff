use ruff_macros::{ViolationMetadata, derive_message_formats};

use crate::Violation;
use crate::codes::Category;

/// ## What it does
/// This is not a regular diagnostic; instead, it's raised when a file cannot be read
/// from disk.
///
/// ## Why is this bad?
/// An `IOError` indicates an error in the development setup. For example, the user may
/// not have permissions to read a given file, or the filesystem may contain a broken
/// symlink.
///
/// ## Example
/// On Linux or macOS:
/// ```shell
/// $ echo 'print("hello world!")' > a.py
/// $ chmod 000 a.py
/// $ ruff a.py
/// a.py:1:1: E902 Permission denied (os error 13)
/// Found 1 error.
/// ```
///
/// ## References
/// - [UNIX Permissions introduction](https://mason.gmu.edu/~montecin/UNIXpermiss.htm)
/// - [Command Line Basics: Symbolic Links](https://www.digitalocean.com/community/tutorials/workflow-symbolic-links)
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "v0.0.28", category = Category::Correctness)]
pub struct IOError {
    pub message: String,
}

/// E902
impl Violation for IOError {
    // The format message is used by the `derive_message_formats` macro.
    #![allow(clippy::useless_format)]
    #[derive_message_formats]
    fn message(&self) -> String {
        let IOError { message } = self;
        format!("{message}")
    }
}
