//! `molt update` and `molt remove` (specs/020-self.md): molt's own copy, replaced only by
//! a release the maintainer signed, or taken off the machine. The network and the crypto
//! are owning tools' (`curl`, `sha256sum`, `ssh-keygen`, `tar`, P-07).

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::rules::find_tool;
use crate::safe_delete::{Outcome, Uid};
use crate::scan::human_size;

#[cfg(test)]
mod tests;

pub const RELEASES: &str = "https://github.com/sumdahl/molt/releases";
/// SE-01: the one build each release publishes.
const TARGET: &str = "x86_64-unknown-linux-musl";
/// The maintainers' keys, built in (`.github/allowed_signers`).
const SIGNERS: &str = include_str!("../.github/allowed_signers");
const NAMESPACE: &str = "molt-release";
/// SE-01: no download is let grow past this.
const MAX_BYTES: &str = "104857600";

/// SE-09: only a debug build, which tests use, takes a test key.
fn signers() -> Cow<'static, str> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("MOLT_TEST_ALLOWED_SIGNERS") {
        if let Ok(text) = fs::read_to_string(path) {
            return Cow::Owned(text);
        }
    }
    Cow::Borrowed(SIGNERS)
}

/// The names an allowed-signers file lets sign: each line's first field.
fn principals(signers: &str) -> Vec<&str> {
    signers
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

/// SE-04: the running binary, when it's yours to replace or remove: under your home,
/// and it and its folder owned by you. `Err` says why not and what to use instead.
pub fn own_binary(home: &Path, euid: Uid) -> Result<PathBuf, String> {
    let exe = fs::read_link("/proc/self/exe")
        .map_err(|error| format!("can't tell where molt is installed: {error}"))?;
    let owner = |path: &Path| fs::symlink_metadata(path).map(|meta| meta.uid()).ok();
    let mine = exe.starts_with(home)
        && owner(&exe) == Some(euid.as_raw())
        && exe.parent().and_then(owner) == Some(euid.as_raw());
    if mine {
        Ok(exe)
    } else {
        Err(format!(
            "molt at {} isn't a copy you own in your home, so whatever installed it \
             (pacman, apt, …) updates and removes it",
            exe.display()
        ))
    }
}

/// The tools an update needs, found on the absolute `$PATH` entries (SC-10).
pub struct Tools {
    curl: PathBuf,
    sha256sum: PathBuf,
    ssh_keygen: PathBuf,
    tar: PathBuf,
    search_path: Vec<PathBuf>,
}

impl Tools {
    pub fn find(search_path: &[PathBuf]) -> Result<Self, String> {
        let find = |tool: &str| find_tool(tool, search_path).ok_or(tool.to_owned());
        let found = (|| {
            Ok::<_, String>(Self {
                curl: find("curl")?,
                sha256sum: find("sha256sum")?,
                ssh_keygen: find("ssh-keygen")?,
                tar: find("tar")?,
                search_path: search_path.to_vec(),
            })
        })();
        found.map_err(|tool| format!("`molt update` needs {tool}, which isn't installed"))
    }

    fn run(&self, program: &Path, args: &[&OsStr], dir: &Path, stdin: Stdio) -> Option<Output> {
        Command::new(program)
            .args(args)
            .current_dir(dir)
            .env(
                "PATH",
                std::env::join_paths(&self.search_path).unwrap_or_default(),
            )
            .stdin(stdin)
            .output()
            .ok()
    }

    fn curl_args(extra: &[&'static str]) -> Vec<&'static OsStr> {
        [
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--fail",
            "--silent",
            "--show-error",
        ]
        .iter()
        .chain(extra)
        .map(|arg| OsStr::new(*arg))
        .collect()
    }

    /// SE-01: the tag `.../releases/latest` redirects to, not followed.
    pub fn latest(&self) -> Result<String, String> {
        let url = format!("{RELEASES}/latest");
        let mut args = Self::curl_args(&["--head", "--output", "/dev/null", "--write-out"]);
        args.extend([OsStr::new("%{redirect_url}"), OsStr::new(&url)]);
        let output = self
            .run(&self.curl, &args, Path::new("/"), Stdio::null())
            .filter(|output| output.status.success())
            .ok_or("can't reach the releases on GitHub")?;
        let target = String::from_utf8_lossy(&output.stdout);
        target
            .trim()
            .strip_prefix(&format!("{RELEASES}/tag/"))
            .filter(|tag| version_of(tag).is_some())
            .map(str::to_owned)
            .ok_or_else(|| "found no release on GitHub".to_owned())
    }

    fn download(&self, url: &str, out: &Path, dir: &Path) -> bool {
        let mut args = Self::curl_args(&["--location", "--max-filesize", MAX_BYTES, "--output"]);
        args.extend([out.as_os_str(), OsStr::new(url)]);
        self.run(&self.curl, &args, dir, Stdio::null())
            .is_some_and(|output| output.status.success())
    }
}

/// `vX.Y.Z` as numbers.
fn version_of(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag
        .strip_prefix('v')?
        .split('.')
        .map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether `tag` is a newer release than `current` (`X.Y.Z`).
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (version_of(tag), version_of(&format!("v{current}"))) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

/// One check of SE-02, as the preview shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub what: String,
    pub ok: bool,
}

/// A release downloaded and checked in its own folder (SE-01, SE-02).
pub struct Prepared {
    pub tag: String,
    pub url: String,
    /// The folder to remove afterwards (SE-06).
    pub folder: PathBuf,
    pub steps: Vec<Step>,
    /// The new binary, when every check passed.
    pub binary: Option<PathBuf>,
}

/// SE-01: a new folder only molt can use, never one that was already there.
fn fresh_folder() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let folder = std::env::temp_dir().join(format!("molt-update-{}-{nanos}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&folder)?;
    Ok(folder)
}

/// SE-01, SE-02: downloads `tag` and runs each check in order, stopping at the first
/// that fails.
pub fn prepare(tools: &Tools, tag: &str) -> std::io::Result<Prepared> {
    let folder = fresh_folder()?;
    let top = format!("molt-{tag}-{TARGET}");
    let tarball = format!("{top}.tar.gz");
    let url = format!("{RELEASES}/download/{tag}/{tarball}");
    let mut prepared = Prepared {
        tag: tag.to_owned(),
        url: url.clone(),
        folder: folder.clone(),
        steps: Vec::new(),
        binary: None,
    };
    let checksum = format!("{tarball}.sha256");
    let signature = format!("{checksum}.sig");
    let checks: [(&str, &dyn Fn() -> bool); 7] = [
        ("download the release, its checksum and signature", &|| {
            [&tarball, &checksum, &signature].iter().all(|name| {
                let from = format!("{RELEASES}/download/{tag}/{name}");
                tools.download(&from, &folder.join(name), &folder)
            })
        }),
        ("the checksum is signed by a molt maintainer", &|| {
            verify_signature(tools, &folder, &checksum, &signature)
        }),
        ("the checksum names this release's file", &|| {
            fs::read_to_string(folder.join(&checksum)).is_ok_and(|text| names_only(&text, &tarball))
        }),
        ("the file matches its checksum (sha256sum)", &|| {
            let args = [
                OsStr::new("--check"),
                OsStr::new("--status"),
                OsStr::new(&checksum),
            ];
            tools
                .run(&tools.sha256sum, &args, &folder, Stdio::null())
                .is_some_and(|output| output.status.success())
        }),
        ("the archive holds only plain files in one folder", &|| {
            safe_archive(tools, &folder, &tarball, &top)
        }),
        ("unpack it", &|| {
            let args = [
                OsStr::new("-xzf"),
                OsStr::new(&tarball),
                OsStr::new("--no-same-owner"),
            ];
            tools
                .run(&tools.tar, &args, &folder, Stdio::null())
                .is_some_and(|output| output.status.success())
                && fs::symlink_metadata(folder.join(&top).join("molt"))
                    .is_ok_and(|meta| meta.file_type().is_file())
        }),
        ("the new molt runs and says it's this version", &|| {
            let binary = folder.join(&top).join("molt");
            let version = tag.trim_start_matches('v');
            tools
                .run(&binary, &[OsStr::new("--version")], &folder, Stdio::null())
                .is_some_and(|output| {
                    String::from_utf8_lossy(&output.stdout).trim() == format!("molt {version}")
                })
        }),
    ];
    for (what, check) in checks {
        let ok = check();
        prepared.steps.push(Step {
            what: what.to_owned(),
            ok,
        });
        if !ok {
            return Ok(prepared);
        }
    }
    prepared.binary = Some(folder.join(&top).join("molt"));
    Ok(prepared)
}

/// SE-02: `ssh-keygen -Y verify` against the allowed signers, for any of their names.
fn verify_signature(tools: &Tools, folder: &Path, checksum: &str, signature: &str) -> bool {
    let signers = signers();
    let file = folder.join("allowed_signers");
    if fs::write(&file, signers.as_bytes()).is_err() {
        return false;
    }
    principals(&signers).into_iter().any(|principal| {
        let Ok(data) = fs::File::open(folder.join(checksum)) else {
            return false;
        };
        let args = [
            OsStr::new("-Y"),
            OsStr::new("verify"),
            OsStr::new("-f"),
            file.as_os_str(),
            OsStr::new("-I"),
            OsStr::new(principal),
            OsStr::new("-n"),
            OsStr::new(NAMESPACE),
            OsStr::new("-s"),
            OsStr::new(signature),
        ];
        tools
            .run(&tools.ssh_keygen, &args, folder, Stdio::from(data))
            .is_some_and(|output| output.status.success())
    })
}

/// SE-02: one line, a sha256 and exactly `tarball`'s name.
fn names_only(checksum: &str, tarball: &str) -> bool {
    let lines: Vec<&str> = checksum
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let [line] = lines.as_slice() else {
        return false;
    };
    let mut fields = line.split_whitespace();
    let (Some(hash), Some(name), None) = (fields.next(), fields.next(), fields.next()) else {
        return false;
    };
    hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && name.trim_start_matches('*') == tarball
}

/// SE-02: every entry is a regular file or folder inside `top`.
fn safe_archive(tools: &Tools, folder: &Path, tarball: &str, top: &str) -> bool {
    let list = |flags: &str| {
        tools
            .run(
                &tools.tar,
                &[OsStr::new(flags), OsStr::new(tarball)],
                folder,
                Stdio::null(),
            )
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let (Some(names), Some(long)) = (list("-tzf"), list("-tvzf")) else {
        return false;
    };
    entries_are_safe(&names, &long, top)
}

fn entries_are_safe(names: &str, long: &str, top: &str) -> bool {
    let inside = |name: &str| {
        let name = name.trim_end_matches('/');
        (name == top || name.starts_with(&format!("{top}/")))
            && !name.split('/').any(|part| part == ".." || part.is_empty())
    };
    let plain = |line: &str| line.starts_with('-') || line.starts_with('d');
    names.lines().count() > 0
        && names.lines().all(inside)
        && long.lines().all(plain)
        && names.lines().count() == long.lines().count()
}

/// SE-07: what `molt remove` takes, as far as it exists: the binary (when SE-04 allows),
/// molt's config, cache and state (without the log unless `with_log`), and the
/// completion files README names.
pub fn leftovers(binary: Option<&Path>, dirs: &crate::rules::Dirs, with_log: bool) -> Vec<PathBuf> {
    let state = dirs.state.join("molt");
    let mut paths: Vec<PathBuf> = binary.map(Path::to_path_buf).into_iter().collect();
    paths.extend([dirs.config.join("molt"), dirs.cache.join("molt")]);
    if with_log {
        paths.push(state);
    } else if let Ok(entries) = fs::read_dir(&state) {
        let mut inside: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.file_name() != Some(OsStr::new("operations.log")))
            .collect();
        inside.sort();
        paths.extend(inside);
    }
    paths.extend([
        dirs.data.join("bash-completion/completions/molt"),
        dirs.home.join(".zfunc/_molt"),
        dirs.config.join("fish/completions/molt.fish"),
    ]);
    paths.retain(|path| fs::symlink_metadata(path).is_ok());
    paths
}

/// A safe-delete outcome as one line of a preview or a result.
pub fn outcome_line(path: &Path, outcome: &Outcome) -> String {
    let (what, bytes) = match *outcome {
        Outcome::WouldTrash { bytes } => ("would trash", Some(bytes)),
        Outcome::Trashed { bytes } => ("trashed", Some(bytes)),
        Outcome::WouldRemove { bytes, .. } => ("would delete", Some(bytes)),
        Outcome::Removed { bytes, .. } => ("deleted", Some(bytes)),
        Outcome::Skipped(reason) => return format!("  kept ({reason})  {}", path.display()),
        Outcome::Failed { errno, .. } => {
            return format!("  failed ({errno})  {}", path.display());
        }
    };
    let size = bytes.map(human_size).unwrap_or_default();
    format!("  {what:<12} {size:>10}  {}", path.display())
}
