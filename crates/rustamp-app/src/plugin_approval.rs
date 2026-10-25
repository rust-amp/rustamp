// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! `rustamp --approve-plugin <file>`: the operator's way to put a plug-in on the
//! allowlist.
//!
//! The approval store (`plugins-approved.toml`, beside the configuration file)
//! is the one thing standing between a DLL on the disk and its code running in
//! the plug-in host, and **nothing in the player ever writes it on its own**
//! (see `rustamp_prefs::plugins`). Until now an operator had to write the
//! entries by hand. This command is that act made usable, and nothing more:
//!
//! * it is an explicit command line, never a UI step and never a consequence of
//!   another action;
//! * it reads the file as bytes, runs the same passive PE preflight the host
//!   runs, and records **the exact image** (SHA-256, size, machine); a changed
//!   file is a new, unapproved image again;
//! * it prints what it approved, so the operator sees the hash, and it does
//!   **not** run, map or load the file.
//!
//! Approving is a decision that this code will run on the user's machine, with
//! the user's rights. The command says so in its output.

use std::path::{Path, PathBuf};

use rustamp_plughost::ApprovalStore;

/// What an approval recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    /// The store the entry went into.
    pub store: PathBuf,
    /// The name recorded (the file name).
    pub name: String,
    /// SHA-256 of the whole image, lowercase hex.
    pub sha256: String,
    /// Size of the image in bytes.
    pub size: u64,
    /// Whether the image was approved already (the entry is refreshed).
    pub already_approved: bool,
}

/// Why an approval did not happen.
#[derive(Debug)]
pub enum ApproveError {
    /// The file could not be read.
    Read(PathBuf, std::io::Error),
    /// The file has no usable name.
    NoName(PathBuf),
    /// The store could not be opened, or the preflight refused the image.
    Store(String),
}

impl std::fmt::Display for ApproveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(path, error) => write!(f, "cannot read {}: {error}", path.display()),
            Self::NoName(path) => write!(f, "{} has no file name", path.display()),
            Self::Store(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ApproveError {}

/// Where the store lives for a configuration file: beside it (the same rule as
/// `App::plugin_approvals_path`).
pub fn store_path_for(config: &Path) -> PathBuf {
    config
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(rustamp_prefs::APPROVALS_FILE)
}

/// Records `file` in the store at `store`.
pub fn approve_file(store: &Path, file: &Path) -> Result<Approved, ApproveError> {
    let bytes =
        std::fs::read(file).map_err(|error| ApproveError::Read(file.to_path_buf(), error))?;
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApproveError::NoName(file.to_path_buf()))?
        .to_string();
    let mut approvals = ApprovalStore::open_or_empty(store)
        .map_err(|error| ApproveError::Store(format!("{}: {error}", store.display())))?;
    let before = approvals.len();
    let entry = approvals
        .approve_bytes(&name, &bytes)
        .map_err(|error| ApproveError::Store(format!("{name}: {error}")))?;
    if let Some(dir) = store.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|error| ApproveError::Store(format!("{}: {error}", dir.display())))?;
    }
    approvals
        .save_to_file(store)
        .map_err(|error| ApproveError::Store(format!("{}: {error}", store.display())))?;
    Ok(Approved {
        store: store.to_path_buf(),
        name,
        sha256: entry.sha256,
        size: entry.size,
        already_approved: approvals.len() == before,
    })
}

/// The text the command prints for an approval.
pub fn report(approved: &Approved) -> String {
    format!(
        "approved {} ({} bytes, sha256 {}){}\n\
         in {}\n\
         This image may now run inside the plug-in host with your rights. A changed \
         file is not covered; approve it again.",
        approved.name,
        approved.size,
        approved.sha256,
        if approved.already_approved {
            " - it was approved before"
        } else {
            ""
        },
        approved.store.display()
    )
}

/// Runs the command for `args` (the arguments after `--approve-plugin`).
///
/// Returns the text to print and the process exit code (0 on success, 1 when
/// any file was refused, 2 for a command line without a file).
pub fn run(config: Option<&Path>, args: &[String]) -> (String, u8) {
    if args.is_empty() {
        return (
            "usage: rustamp --approve-plugin <plug-in.dll> [<plug-in.dll> ...]".to_string(),
            2,
        );
    }
    let Some(config) = config else {
        return (
            "cannot tell where the approval store belongs: neither %RUSTAMP_CONFIG% nor %APPDATA% is set"
                .to_string(),
            1,
        );
    };
    let store = store_path_for(config);
    let mut lines = Vec::new();
    let mut code = 0;
    for file in args {
        match approve_file(&store, Path::new(file)) {
            Ok(approved) => lines.push(report(&approved)),
            Err(error) => {
                lines.push(format!("not approved: {error}"));
                code = 1;
            }
        }
    }
    (lines.join("\n"), code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rustamp-approve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_store_sits_beside_the_configuration_file() {
        let store = store_path_for(Path::new("C:/cfg/rustamp.toml"));
        assert_eq!(
            store,
            Path::new("C:/cfg").join(rustamp_prefs::APPROVALS_FILE)
        );
        // A bare file name means the current folder.
        assert_eq!(
            store_path_for(Path::new("rustamp.toml")),
            PathBuf::from(rustamp_prefs::APPROVALS_FILE)
        );
    }

    #[test]
    fn a_file_that_is_not_a_plugin_is_refused_and_nothing_is_written() {
        let dir = temp_dir("refuse");
        let junk = dir.join("vis_junk.dll");
        std::fs::write(&junk, b"this is not a PE image").unwrap();
        let store = dir.join(rustamp_prefs::APPROVALS_FILE);

        let error = approve_file(&store, &junk).expect_err("the preflight must refuse it");
        assert!(matches!(error, ApproveError::Store(_)), "{error}");
        assert!(!store.exists(), "a refusal writes no store");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let dir = temp_dir("missing");
        let error = approve_file(
            &dir.join(rustamp_prefs::APPROVALS_FILE),
            &dir.join("nothing.dll"),
        )
        .expect_err("there is no such file");
        assert!(matches!(error, ApproveError::Read(..)), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_command_without_a_file_prints_its_usage() {
        let (text, code) = run(Some(Path::new("rustamp.toml")), &[]);
        assert_eq!(code, 2);
        assert!(text.starts_with("usage:"), "{text}");
        let (text, code) = run(None, &["x.dll".to_string()]);
        assert_eq!(code, 1);
        assert!(text.contains("approval store"), "{text}");
    }

    #[test]
    fn the_report_names_the_image_and_says_what_approval_means() {
        let text = report(&Approved {
            store: PathBuf::from("C:/cfg/plugins-approved.toml"),
            name: "vis_x.dll".to_string(),
            sha256: "ab".repeat(32),
            size: 1234,
            already_approved: false,
        });
        assert!(
            text.contains("vis_x.dll") && text.contains("1234"),
            "{text}"
        );
        assert!(text.contains(&"ab".repeat(32)), "{text}");
        assert!(text.contains("with your rights"), "{text}");
        assert!(text.contains("approve it again"), "{text}");
    }
}
