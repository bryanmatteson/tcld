//! Path resolution for the debugger nub: tclsh, libtcld_dbg, bootstrap.
//!
//! `tcld debug` drives the nub over the wire protocol in a tclsh
//! subprocess, so it needs no compile-time Tcl linkage — only three
//! runtime pieces. Each resolves as: explicit flag → environment
//! variable → discovery. When the nub dylib cannot be found the error
//! is the defined capability diagnostic (see COMPAT.md): release archives
//! bundle both ABIs, while source/custom installs may need an explicit nub.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tcld_debug_wire::PROTOCOL_MAJOR;

/// Nub bootstrap script embedded at build time so installed binaries
/// work outside a workspace checkout.
const EMBEDDED_BOOTSTRAP: &str =
    include_str!("../../../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
const TCLSH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const NUB_MANIFEST_SCHEMA: u32 = 1;
const TARGET_TRIPLE: &str = env!("TCLD_TARGET_TRIPLE");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NubManifest {
    schema_version: u32,
    tcl_abi: String,
    wire_protocol_major: u32,
    target_triple: String,
    library_filename: String,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TclAbi {
    Tcl85,
    Tcl86,
    Tcl9,
}

impl TclAbi {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcl85 => "Tcl 8.5",
            Self::Tcl86 => "Tcl 8.6",
            Self::Tcl9 => "Tcl 9",
        }
    }

    pub fn bundle_dir(self) -> &'static str {
        match self {
            Self::Tcl85 => "tcl8.5",
            Self::Tcl86 => "tcl8.6",
            Self::Tcl9 => "tcl9.0",
        }
    }

    pub fn manifest_value(self) -> &'static str {
        self.bundle_dir()
    }

    fn versioned_shell_names(self) -> &'static [&'static str] {
        match self {
            Self::Tcl85 => &["tclsh8.5"],
            Self::Tcl86 => &["tclsh8.6"],
            Self::Tcl9 => &["tclsh9.0", "tclsh9"],
        }
    }

    fn prefix_env_key(self) -> &'static str {
        match self {
            Self::Tcl85 => "TCL8_5_PREFIX",
            Self::Tcl86 => "TCL8_6_PREFIX",
            Self::Tcl9 => "TCL9_PREFIX",
        }
    }

    /// Preferred discovery order for a required (or unrestricted) ABI.
    ///
    /// Tcl 8.x bytecode targets prefer 8.6 but may fall back to 8.5 so
    /// stock macOS (`/usr/bin/tclsh` → 8.5.9) can still host the debugger
    /// when a matching 8.5 nub is present.
    fn desired_search_order(required: Option<Self>) -> Vec<Self> {
        match required {
            Some(Self::Tcl9) => vec![Self::Tcl9],
            Some(Self::Tcl86) => vec![Self::Tcl86, Self::Tcl85],
            Some(Self::Tcl85) => vec![Self::Tcl85, Self::Tcl86],
            None => vec![Self::Tcl9, Self::Tcl86, Self::Tcl85],
        }
    }
}

impl std::fmt::Display for TclAbi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub struct NubPaths {
    pub tclsh: PathBuf,
    pub tcl_patchlevel: String,
    pub tcl_abi: Option<TclAbi>,
    pub bootstrap: PathBuf,
    pub dylib: PathBuf,
    pub selection_warning: Option<String>,
    /// Keep-alive guard for a temp dir holding the embedded bootstrap
    /// copy; `None` when the bootstrap came from disk.
    _bootstrap_guard: Option<Arc<tempfile::TempDir>>,
}

impl NubPaths {
    /// Bundle explicitly-known paths (tests and callers that already
    /// resolved discovery themselves).
    pub fn explicit(tclsh: PathBuf, bootstrap: PathBuf, dylib: PathBuf) -> Self {
        Self {
            tclsh,
            tcl_patchlevel: "unprobed".to_string(),
            tcl_abi: None,
            bootstrap,
            dylib,
            selection_warning: None,
            _bootstrap_guard: None,
        }
    }
}

pub fn locate_paths(
    tclsh_override: Option<&str>,
    dylib_override: Option<&str>,
    bootstrap_override: Option<&str>,
) -> Result<NubPaths, String> {
    locate_paths_for_target(tclsh_override, dylib_override, bootstrap_override, None)
}

pub fn locate_paths_for_target(
    tclsh_override: Option<&str>,
    dylib_override: Option<&str>,
    bootstrap_override: Option<&str>,
    required_tcl_abi: Option<TclAbi>,
) -> Result<NubPaths, String> {
    let (bootstrap, guard) = locate_bootstrap(bootstrap_override)?;
    let shell = locate_tclsh(tclsh_override, required_tcl_abi)?;
    Ok(NubPaths {
        tclsh: shell.path,
        tcl_patchlevel: shell.patchlevel,
        tcl_abi: Some(shell.abi),
        bootstrap,
        dylib: locate_dylib(dylib_override, shell.abi)?,
        selection_warning: shell.warning,
        _bootstrap_guard: guard,
    })
}

#[derive(Debug)]
struct ShellSelection {
    path: PathBuf,
    patchlevel: String,
    abi: TclAbi,
    warning: Option<String>,
}

fn locate_tclsh(
    override_path: Option<&str>,
    required_tcl_abi: Option<TclAbi>,
) -> Result<ShellSelection, String> {
    let env_override = std::env::var("TCLD_TCLSH").ok();
    let selected_override = override_path
        .map(|value| (value.to_string(), "--tclsh"))
        .or_else(|| env_override.map(|value| (value, "TCLD_TCLSH")));
    let path_var = std::env::var_os("PATH");
    locate_tclsh_in_with_prefixes(
        selected_override,
        required_tcl_abi,
        path_var.as_deref(),
        &runtime_tcl_prefixes(),
    )
}

#[cfg(test)]
fn locate_tclsh_in(
    selected_override: Option<(String, &'static str)>,
    required_tcl_abi: Option<TclAbi>,
    path_var: Option<&OsStr>,
) -> Result<ShellSelection, String> {
    locate_tclsh_in_with_prefixes(selected_override, required_tcl_abi, path_var, &[])
}

fn locate_tclsh_in_with_prefixes(
    selected_override: Option<(String, &'static str)>,
    required_tcl_abi: Option<TclAbi>,
    path_var: Option<&OsStr>,
    prefixes: &[(Option<TclAbi>, PathBuf)],
) -> Result<ShellSelection, String> {
    if let Some((raw, source)) = selected_override {
        let path = resolve_executable_in(&raw, path_var).unwrap_or_else(|| PathBuf::from(&raw));
        let patchlevel = probe_tclsh(&path).map_err(|error| {
            format!(
                "cannot use explicit Tcl shell from {source} (`{raw}`): {error}. \
                 Choose a Tcl 8.5, 8.6, or 9 shell with `--tclsh <path>` or TCLD_TCLSH=<path>."
            )
        })?;
        let abi = classify_patchlevel(&patchlevel)?;
        let warning = required_tcl_abi
            .filter(|required| *required != abi)
            .map(|required| {
                format!(
                    "warning: bytecode target prefers {required}, but explicit shell override \
                 `{raw}` selected Tcl {patchlevel} ({abi}); execution may fail because the \
                 override crosses the artifact ABI"
                )
            });
        return Ok(ShellSelection {
            path,
            patchlevel,
            abi,
            warning,
        });
    }

    let desired = TclAbi::desired_search_order(required_tcl_abi);
    let mut searched = Vec::new();
    // Walk preferred ABIs in order. Accept only when the probed shell's
    // major.minor matches the ABI slot currently under consideration so a
    // bare `tclsh` that is Apple's 8.5 cannot short-circuit Tcl 9 / 8.6
    // discovery earlier in the list.
    for abi in &desired {
        let mut candidates = Vec::new();
        for name in abi.versioned_shell_names() {
            if let Some(path) = resolve_executable_in(name, path_var) {
                candidates.push(path);
            } else {
                searched.push(format!("{name} (not found)"));
            }
        }
        for (_, prefix) in prefixes
            .iter()
            .filter(|(prefix_abi, _)| prefix_abi.is_none_or(|value| value == *abi))
        {
            for name in abi.versioned_shell_names() {
                let path = prefix.join("bin").join(name);
                if path.is_file() && !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
        }
        if let Some(path) = resolve_executable_in("tclsh", path_var)
            && !candidates.contains(&path)
        {
            candidates.push(path);
        }
        for path in candidates {
            match probe_tclsh(&path) {
                Ok(patchlevel) => match classify_patchlevel(&patchlevel) {
                    Ok(actual) if actual == *abi => {
                        let warning = required_tcl_abi.filter(|required| *required != actual).map(
                            |required| {
                                format!(
                                    "warning: preferred runtime is {required}, but selected \
                                     Tcl {patchlevel} ({actual}) via {}; capabilities may be \
                                     reduced and some scripts may fail",
                                    path.display()
                                )
                            },
                        );
                        return Ok(ShellSelection {
                            path,
                            patchlevel,
                            abi: actual,
                            warning,
                        });
                    }
                    Ok(actual) => searched.push(format!(
                        "{} -> Tcl {patchlevel} ({actual}, expected {abi})",
                        path.display()
                    )),
                    Err(error) => searched.push(format!("{} -> {error}", path.display())),
                },
                Err(error) => searched.push(format!("{} -> {error}", path.display())),
            }
        }
    }
    let requirement = required_tcl_abi
        .map(|abi| match abi {
            TclAbi::Tcl9 => "Tcl 9".to_string(),
            TclAbi::Tcl86 => "Tcl 8.6 (or 8.5 fallback)".to_string(),
            TclAbi::Tcl85 => "Tcl 8.5 (or 8.6)".to_string(),
        })
        .unwrap_or_else(|| "Tcl 9, 8.6, or 8.5".to_string());
    Err(format!(
        "no supported Tcl shell found for {requirement}. Searched: {}. \
         Install a matching shell or pass `--tclsh <path>` / `TCLD_TCLSH=<path>`.",
        searched.join(", ")
    ))
}

fn runtime_tcl_prefixes() -> Vec<(Option<TclAbi>, PathBuf)> {
    let mut prefixes = Vec::new();
    for abi in [TclAbi::Tcl9, TclAbi::Tcl86, TclAbi::Tcl85] {
        if let Some(prefix) = std::env::var_os(abi.prefix_env_key()) {
            prefixes.push((Some(abi), PathBuf::from(prefix)));
        }
    }
    if let Some(prefix) = std::env::var_os("TCL_PREFIX") {
        prefixes.push((None, PathBuf::from(prefix)));
    }
    for (abi, prefix) in [
        (TclAbi::Tcl9, "/opt/homebrew/opt/tcl-tk"),
        (TclAbi::Tcl9, "/opt/homebrew/opt/tcl-tk@9"),
        (TclAbi::Tcl9, "/usr/local/opt/tcl-tk"),
        (TclAbi::Tcl9, "/usr/local/opt/tcl-tk@9"),
        (TclAbi::Tcl86, "/opt/homebrew/opt/tcl-tk@8"),
        (TclAbi::Tcl86, "/usr/local/opt/tcl-tk@8"),
        (TclAbi::Tcl86, "/opt/local"),
        (TclAbi::Tcl9, "/opt/local"),
    ] {
        prefixes.push((Some(abi), PathBuf::from(prefix)));
    }
    prefixes
}

fn find_in_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn resolve_executable_in(value: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    if path.components().count() > 1 {
        return path.is_file().then_some(path);
    }
    find_in_path(value, path_var)
}

fn probe_tclsh(path: &Path) -> Result<String, String> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", path.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(b"puts -nonewline [info patchlevel]\n")
            .map_err(|error| format!("failed to probe {}: {error}", path.display()))?;
    }
    let deadline = Instant::now() + TCLSH_PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} timed out after {:?}",
                    path.display(),
                    TCLSH_PROBE_TIMEOUT
                ));
            }
            Err(error) => return Err(format!("probe wait failed: {error}")),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if !status.success() {
        return Err(format!(
            "{} exited with {status}: {}",
            path.display(),
            stderr.trim()
        ));
    }
    let patchlevel = stdout.trim();
    if patchlevel.is_empty() || patchlevel.split('.').count() < 2 {
        return Err(format!(
            "{} returned malformed patchlevel `{patchlevel}`",
            path.display()
        ));
    }
    Ok(patchlevel.to_string())
}

fn classify_patchlevel(patchlevel: &str) -> Result<TclAbi, String> {
    let mut parts = patchlevel.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed Tcl patchlevel `{patchlevel}`"))?;
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed Tcl patchlevel `{patchlevel}`"))?;
    match (major, minor) {
        (9, _) => Ok(TclAbi::Tcl9),
        (8, 6) => Ok(TclAbi::Tcl86),
        (8, 5) => Ok(TclAbi::Tcl85),
        _ => Err(format!(
            "unsupported Tcl patchlevel `{patchlevel}`; tcld debug supports Tcl 8.5, 8.6, and 9"
        )),
    }
}

fn locate_bootstrap(
    override_path: Option<&str>,
) -> Result<(PathBuf, Option<Arc<tempfile::TempDir>>), String> {
    if let Some(p) = override_path {
        let path = PathBuf::from(p);
        if !path.exists() {
            return Err(format!("bootstrap script not found at {}", path.display()));
        }
        return Ok((path, None));
    }
    // Workspace checkout: use the canonical script on disk.
    let p = workspace_root().join("crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
    if p.exists() {
        return Ok((p, None));
    }
    // Installed binary: materialize the embedded copy into a temp dir
    // that lives as long as the debug session.
    let dir = tempfile::TempDir::new()
        .map_err(|e| format!("cannot create temp dir for nub bootstrap: {e}"))?;
    let path = dir.path().join("tcld_nub_bootstrap.tcl");
    std::fs::write(&path, EMBEDDED_BOOTSTRAP)
        .map_err(|e| format!("cannot write nub bootstrap to {}: {e}", path.display()))?;
    Ok((path, Some(Arc::new(dir))))
}

fn locate_dylib(override_path: Option<&str>, abi: TclAbi) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        let path = PathBuf::from(p);
        if !path.exists() {
            return Err(format!("debugger dylib not found at {}", path.display()));
        }
        validate_nub_manifest(&path, abi, false)?;
        return Ok(path);
    }
    if let Ok(p) = std::env::var("TCLD_DBG_DYLIB") {
        let path = PathBuf::from(p);
        if path.exists() {
            validate_nub_manifest(&path, abi, false)?;
            return Ok(path);
        }
    }
    let workspace_root = workspace_root();
    let current_exe = std::env::current_exe().ok();
    let candidates = automatic_dylib_candidates(abi, &workspace_root, current_exe.as_deref());
    for (candidate, bundled) in &candidates {
        if candidate.exists() {
            validate_nub_manifest(candidate, abi, *bundled)?;
            return Ok(candidate.clone());
        }
    }
    Err(nub_unavailable_message())
}

fn automatic_dylib_candidates(
    abi: TclAbi,
    workspace_root: &Path,
    current_exe: Option<&Path>,
) -> Vec<(PathBuf, bool)> {
    let mut bundled = Vec::new();
    let mut workspace = Vec::new();
    let mut adjacent = Vec::new();

    // Installed release archives own precedence over build artifacts left in the
    // checkout that produced them. This also makes archive verification exercise
    // the exact files a user receives.
    if let Some(bin_dir) = current_exe.and_then(Path::parent) {
        for ext in ["dylib", "so"] {
            bundled.push((
                bin_dir
                    .join("lib/tcld/nub")
                    .join(abi.bundle_dir())
                    .join(format!("libtcld_dbg.{ext}")),
                true,
            ));
            bundled.push((
                bin_dir
                    .join("../lib/tcld/nub")
                    .join(abi.bundle_dir())
                    .join(format!("libtcld_dbg.{ext}")),
                true,
            ));
            if abi == TclAbi::Tcl9 {
                adjacent.push((bin_dir.join(format!("libtcld_dbg.{ext}")), false));
                adjacent.push((
                    bin_dir.join("../lib").join(format!("libtcld_dbg.{ext}")),
                    false,
                ));
            }
        }
    }

    for target_dir in match abi {
        TclAbi::Tcl85 => ["release-nub-tcl85", "debug-nub-tcl85"],
        TclAbi::Tcl86 => ["release-nub-tcl86", "debug-nub-tcl86"],
        TclAbi::Tcl9 => ["release-nub-tcl9", "debug-nub-tcl9"],
    } {
        for profile in ["release", "debug"] {
            for ext in ["dylib", "so"] {
                workspace.push((
                    workspace_root
                        .join("target")
                        .join(target_dir)
                        .join(profile)
                        .join(format!("libtcld_dbg.{ext}")),
                    false,
                ));
            }
        }
    }
    // The ordinary Cargo target directory is built with tcld-dbg-ext's
    // default `tcl9` feature. It has no ABI-qualified path or required
    // manifest, so treating it as an 8.5/8.6 candidate can load a Tcl 9
    // extension into an older shell. That may survive the handshake because
    // much of Tcl's public C API is source-compatible, then corrupt frame and
    // trace semantics at runtime. Older ABIs must come from their qualified
    // target directories (or a verified release bundle).
    if abi == TclAbi::Tcl9 {
        for profile in ["debug", "release"] {
            for ext in ["dylib", "so"] {
                workspace.push((
                    workspace_root
                        .join("target")
                        .join(profile)
                        .join(format!("libtcld_dbg.{ext}")),
                    false,
                ));
            }
        }
    }
    bundled.extend(workspace);
    bundled.extend(adjacent);
    bundled
}

fn validate_nub_manifest(path: &Path, abi: TclAbi, required: bool) -> Result<(), String> {
    let manifest_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("nub-manifest.json");
    if !manifest_path.exists() {
        if required {
            return Err(format!(
                "bundled debugger nub {} is missing required manifest {}",
                path.display(),
                manifest_path.display()
            ));
        }
        return Ok(());
    }
    let bytes = std::fs::read(&manifest_path)
        .map_err(|error| format!("read {}: {error}", manifest_path.display()))?;
    let manifest: NubManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid nub manifest {}: {error}", manifest_path.display()))?;
    if manifest.schema_version != NUB_MANIFEST_SCHEMA {
        return Err(format!(
            "nub manifest {} has schema {}, expected {}",
            manifest_path.display(),
            manifest.schema_version,
            NUB_MANIFEST_SCHEMA
        ));
    }
    if manifest.tcl_abi != abi.manifest_value() {
        return Err(format!(
            "nub manifest {} targets {}, selected shell requires {}",
            manifest_path.display(),
            manifest.tcl_abi,
            abi.manifest_value()
        ));
    }
    if manifest.wire_protocol_major != PROTOCOL_MAJOR {
        return Err(format!(
            "nub manifest {} uses wire protocol {}, tcld requires {}",
            manifest_path.display(),
            manifest.wire_protocol_major,
            PROTOCOL_MAJOR
        ));
    }
    if manifest.target_triple != TARGET_TRIPLE {
        return Err(format!(
            "nub manifest {} targets {}, current CLI targets {}",
            manifest_path.display(),
            manifest.target_triple,
            TARGET_TRIPLE
        ));
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if manifest.library_filename != filename {
        return Err(format!(
            "nub manifest {} names library `{}`, found `{filename}`",
            manifest_path.display(),
            manifest.library_filename
        ));
    }
    let library = std::fs::read(path)
        .map_err(|error| format!("read debugger nub {}: {error}", path.display()))?;
    validate_library_format(path, &library)?;
    let actual = format!("{:x}", Sha256::digest(&library));
    if !manifest.sha256.eq_ignore_ascii_case(&actual) {
        return Err(format!(
            "nub manifest {} checksum mismatch: expected {}, got {}",
            manifest_path.display(),
            manifest.sha256,
            actual
        ));
    }
    Ok(())
}

fn validate_library_format(path: &Path, library: &[u8]) -> Result<(), String> {
    let extension = path.extension().and_then(|value| value.to_str());
    let valid = match extension {
        Some("so") => library.starts_with(b"\x7fELF"),
        Some("dylib") => {
            let magic = library.get(..4);
            [
                b"\xfe\xed\xfa\xce".as_slice(),
                b"\xce\xfa\xed\xfe".as_slice(),
                b"\xfe\xed\xfa\xcf".as_slice(),
                b"\xcf\xfa\xed\xfe".as_slice(),
                b"\xca\xfe\xba\xbe".as_slice(),
                b"\xbe\xba\xfe\xca".as_slice(),
                b"\xca\xfe\xba\xbf".as_slice(),
                b"\xbf\xba\xfe\xca".as_slice(),
            ]
            .contains(&magic.unwrap_or_default())
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "debugger nub {} is not a valid ELF shared object or Mach-O dynamic library",
            path.display()
        ))
    }
}

/// The defined diagnostic for a build/install without the debugger nub
/// (COMPAT.md capability contract). Kept as a function so the contract
/// test asserts the exact remediation content.
pub fn nub_unavailable_message() -> String {
    "runtime debugger unavailable: the debugger nub (libtcld_dbg.{dylib,so}) was not found.\n\
     Official release archives bundle Tcl 8.5, 8.6, and 9 nubs under lib/tcld/nub when available.\n\
     For a source or custom install:\n\
       1. build a matching loadable nub, e.g.\n\
          `cargo build -p tcld-dbg-ext --no-default-features --features tcl8_5,loadable-extension`\n\
          (or tcl8_6 / tcl9), then\n\
       2. point tcld at it with `--dylib <path>` or `TCLD_DBG_DYLIB=<path>`.\n\
     A tclsh 8.5, 8.6, or 9 is also required (`--tclsh <path>` or `TCLD_TCLSH=<path>`).\n\
     All other tcld commands (decompile, disassemble, dump, explore) work without the nub."
        .to_string()
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../..").canonicalize().unwrap_or(manifest)
}

pub fn ensure_user_script(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("user script not found: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn executable_script(body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("tclsh-test");
        std::fs::write(&shell, format!("#!/bin/sh\nread -r _\n{body}\n")).unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, shell)
    }

    fn manifest_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let (filename, bytes): (&str, &[u8]) = if cfg!(target_os = "macos") {
            ("libtcld_dbg.dylib", b"\xcf\xfa\xed\xfe\0\0\0\0")
        } else {
            ("libtcld_dbg.so", b"\x7fELF\0\0\0\0")
        };
        let library = dir.path().join(filename);
        std::fs::write(&library, bytes).unwrap();
        (dir, library)
    }

    fn write_manifest(library: &Path, mut value: serde_json::Value) {
        let checksum = format!("{:x}", Sha256::digest(std::fs::read(library).unwrap()));
        value["sha256"] = serde_json::Value::String(checksum);
        std::fs::write(
            library.parent().unwrap().join("nub-manifest.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    fn valid_manifest() -> serde_json::Value {
        let filename = if cfg!(target_os = "macos") {
            "libtcld_dbg.dylib"
        } else {
            "libtcld_dbg.so"
        };
        serde_json::json!({
            "schema_version": 1,
            "tcl_abi": "tcl9.0",
            "wire_protocol_major": PROTOCOL_MAJOR,
            "target_triple": TARGET_TRIPLE,
            "library_filename": filename,
            "sha256": "filled-by-test"
        })
    }

    #[test]
    fn missing_nub_reports_actionable_diagnostic() {
        // The capability contract (plan A6): the diagnostic names the
        // build command, both override channels, and the surviving
        // command surface.
        let msg = nub_unavailable_message();
        assert!(msg.contains("cargo build -p tcld-dbg-ext"));
        assert!(msg.contains("--dylib"));
        assert!(msg.contains("TCLD_DBG_DYLIB"));
        assert!(msg.contains("--tclsh"));
        assert!(msg.contains("decompile"));
    }

    #[test]
    fn explicit_dylib_override_must_exist() {
        let err = locate_dylib(Some("/nonexistent/libtcld_dbg.dylib"), TclAbi::Tcl9).unwrap_err();
        assert!(err.contains("/nonexistent/libtcld_dbg.dylib"));
    }

    #[test]
    fn bundled_nubs_precede_workspace_and_adjacent_fallbacks() {
        let workspace = Path::new("/checkout");
        let executable = Path::new("/archive/tcld");
        let candidates = automatic_dylib_candidates(TclAbi::Tcl9, workspace, Some(executable));
        let first_workspace = candidates
            .iter()
            .position(|(path, _)| path.starts_with(workspace))
            .expect("workspace candidate");
        let last_bundled = candidates
            .iter()
            .rposition(|(_, bundled)| *bundled)
            .expect("bundled candidate");
        let first_adjacent = candidates
            .iter()
            .position(|(path, bundled)| !bundled && path.starts_with("/archive"))
            .expect("adjacent fallback");

        assert!(last_bundled < first_workspace);
        assert!(first_workspace < first_adjacent);
        assert!(
            candidates[0]
                .0
                .ends_with("lib/tcld/nub/tcl9.0/libtcld_dbg.dylib")
        );
    }

    #[test]
    fn older_abis_never_auto_select_the_generic_tcl9_target() {
        let workspace = Path::new("/checkout");
        let executable = Path::new("/archive/tcld");

        for abi in [TclAbi::Tcl85, TclAbi::Tcl86] {
            let candidates = automatic_dylib_candidates(abi, workspace, Some(executable));
            assert!(
                !candidates.iter().any(|(path, _)| {
                    path == Path::new("/checkout/target/debug/libtcld_dbg.dylib")
                        || path == Path::new("/checkout/target/release/libtcld_dbg.dylib")
                        || path == Path::new("/checkout/target/debug/libtcld_dbg.so")
                        || path == Path::new("/checkout/target/release/libtcld_dbg.so")
                        || path == Path::new("/archive/libtcld_dbg.dylib")
                        || path == Path::new("/archive/libtcld_dbg.so")
                }),
                "{abi} admitted an ABI-unknown generic target: {candidates:?}"
            );
        }

        let tcl9 = automatic_dylib_candidates(TclAbi::Tcl9, workspace, Some(executable));
        assert!(
            tcl9.iter()
                .any(|(path, _)| { path == Path::new("/checkout/target/debug/libtcld_dbg.dylib") })
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_tclsh_override_is_probed_and_authoritative() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("tclsh-custom");
        std::fs::write(&shell, "#!/bin/sh\nread -r _\nprintf 9.0.3\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let selection = locate_tclsh(shell.to_str(), Some(TclAbi::Tcl86)).unwrap();
        assert_eq!(selection.path, shell);
        assert_eq!(selection.patchlevel, "9.0.3");
        assert_eq!(selection.abi, TclAbi::Tcl9);
        assert!(selection.warning.unwrap().contains("prefers Tcl 8.6"));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_tcl85_is_accepted_as_supported_abi() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("tclsh8.5");
        std::fs::write(&shell, "#!/bin/sh\nread -r _\nprintf 8.5.9\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let selection = locate_tclsh(shell.to_str(), None).expect("8.5 is supported");
        assert_eq!(selection.abi, TclAbi::Tcl85);
        assert_eq!(selection.patchlevel, "8.5.9");
    }

    #[cfg(unix)]
    #[test]
    fn tcl86_required_falls_back_to_tcl85_with_warning() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("tclsh8.5");
        std::fs::write(&shell, "#!/bin/sh\nread -r _\nprintf 8.5.9\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

        let selection =
            locate_tclsh_in(None, Some(TclAbi::Tcl86), Some(dir.path().as_os_str())).unwrap();
        assert_eq!(selection.abi, TclAbi::Tcl85);
        assert_eq!(selection.patchlevel, "8.5.9");
        assert!(
            selection
                .warning
                .as_deref()
                .is_some_and(|text| text.contains("preferred runtime is Tcl 8.6")),
            "{:?}",
            selection.warning
        );
    }

    #[cfg(unix)]
    #[test]
    fn malformed_tclsh_probe_output_is_rejected() {
        let (_dir, shell) = executable_script("printf not-a-version");
        let error = locate_tclsh(shell.to_str(), None).unwrap_err();
        assert!(error.contains("malformed patchlevel"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn tclsh_probe_timeout_is_bounded_and_actionable() {
        let (_dir, shell) = executable_script("sleep 10");
        let started = Instant::now();
        let error = locate_tclsh(shell.to_str(), None).unwrap_err();
        assert!(error.contains("timed out after 2s"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_explicit_tclsh_is_rejected_before_terminal_start() {
        let missing = "/nonexistent/tcld-test-tclsh9.0";
        let error = locate_tclsh(Some(missing), Some(TclAbi::Tcl9)).unwrap_err();
        assert!(error.contains(missing), "{error}");
        // Emulation can report exec failure during the probe, after spawn.
        assert!(error.contains("Choose a Tcl"), "{error}");
    }

    #[test]
    fn bootstrap_falls_back_to_embedded_copy_outside_workspace() {
        // The embedded copy must match the canonical script so installed
        // binaries run the same nub bootstrap as workspace builds.
        let canonical = workspace_root().join("crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
        let on_disk = std::fs::read_to_string(canonical).expect("canonical bootstrap");
        assert_eq!(EMBEDDED_BOOTSTRAP, on_disk);
    }

    #[test]
    fn bundled_nub_manifest_validates_every_contract_field() {
        let (_dir, library) = manifest_fixture();
        write_manifest(&library, valid_manifest());
        validate_nub_manifest(&library, TclAbi::Tcl9, true).unwrap();

        let cases = [
            ("tcl_abi", serde_json::json!("tcl8.6")),
            ("wire_protocol_major", serde_json::json!(1)),
            ("target_triple", serde_json::json!("wrong-target")),
            ("library_filename", serde_json::json!("wrong.dylib")),
        ];
        for (field, replacement) in cases {
            let mut manifest = valid_manifest();
            manifest[field] = replacement;
            write_manifest(&library, manifest);
            let error = validate_nub_manifest(&library, TclAbi::Tcl9, true)
                .err()
                .unwrap();
            assert!(error.contains("manifest"), "{field}: {error}");
        }

        let mut manifest = valid_manifest();
        manifest["sha256"] = serde_json::json!("bad");
        std::fs::write(
            library.parent().unwrap().join("nub-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            validate_nub_manifest(&library, TclAbi::Tcl9, true)
                .err()
                .unwrap()
                .contains("checksum mismatch")
        );
    }

    #[test]
    fn bundled_manifest_is_required_and_unknown_fields_are_rejected() {
        let (_dir, library) = manifest_fixture();
        assert!(
            validate_nub_manifest(&library, TclAbi::Tcl9, true)
                .err()
                .unwrap()
                .contains("missing required manifest")
        );
        assert!(validate_nub_manifest(&library, TclAbi::Tcl9, false).is_ok());

        let mut manifest = valid_manifest();
        manifest["future_field"] = serde_json::json!(true);
        write_manifest(&library, manifest);
        assert!(
            validate_nub_manifest(&library, TclAbi::Tcl9, true)
                .err()
                .unwrap()
                .contains("unknown field")
        );
    }

    #[test]
    fn bundled_manifest_rejects_corrupt_library_before_spawn() {
        let (_dir, library) = manifest_fixture();
        std::fs::write(&library, b"not a native library").unwrap();
        write_manifest(&library, valid_manifest());
        let error = validate_nub_manifest(&library, TclAbi::Tcl9, true).unwrap_err();
        assert!(error.contains("not a valid ELF"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn automatic_selection_never_crosses_required_bytecode_abi() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let tcl9 = dir.path().join("tclsh9.0");
        std::fs::write(&tcl9, "#!/bin/sh\nread -r _\nprintf 9.0.3\n").unwrap();
        std::fs::set_permissions(&tcl9, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error =
            locate_tclsh_in(None, Some(TclAbi::Tcl86), Some(dir.path().as_os_str())).unwrap_err();
        assert!(error.contains("Tcl 8.6"), "{error}");
        assert!(error.contains("tclsh8.6 (not found)"), "{error}");
        assert!(error.contains("--tclsh <path>"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn automatic_selection_prefers_target_abi_then_tcl9_for_source() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        for (name, version) in [
            ("tclsh8.5", "8.5.9"),
            ("tclsh8.6", "8.6.17"),
            ("tclsh9.0", "9.0.3"),
        ] {
            let shell = dir.path().join(name);
            std::fs::write(&shell, format!("#!/bin/sh\nread -r _\nprintf {version}\n")).unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let bytecode =
            locate_tclsh_in(None, Some(TclAbi::Tcl86), Some(dir.path().as_os_str())).unwrap();
        assert_eq!(bytecode.abi, TclAbi::Tcl86);
        assert!(bytecode.path.ends_with("tclsh8.6"));
        assert!(bytecode.warning.is_none());

        let source = locate_tclsh_in(None, None, Some(dir.path().as_os_str())).unwrap();
        assert_eq!(source.abi, TclAbi::Tcl9);
        assert!(source.path.ends_with("tclsh9.0"));
    }

    #[cfg(unix)]
    #[test]
    fn bare_tclsh_85_does_not_shadow_versioned_86() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        // Order on disk does not matter; versioned 8.6 must win over bare 8.5
        // when both satisfy the preferred search order.
        for (name, version) in [("tclsh", "8.5.9"), ("tclsh8.6", "8.6.17")] {
            let shell = dir.path().join(name);
            std::fs::write(&shell, format!("#!/bin/sh\nread -r _\nprintf {version}\n")).unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let selection =
            locate_tclsh_in(None, Some(TclAbi::Tcl86), Some(dir.path().as_os_str())).unwrap();
        assert_eq!(selection.abi, TclAbi::Tcl86);
        assert!(selection.path.ends_with("tclsh8.6"));
    }

    #[cfg(unix)]
    #[test]
    fn versioned_install_prefix_beats_path_fallback() {
        use std::os::unix::fs::PermissionsExt;

        let path_dir = tempfile::tempdir().unwrap();
        let fallback = path_dir.path().join("tclsh8.5");
        std::fs::write(&fallback, "#!/bin/sh\nread -r _\nprintf 8.5.9\n").unwrap();
        std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755)).unwrap();

        let prefix = tempfile::tempdir().unwrap();
        let bin = prefix.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let preferred = bin.join("tclsh8.6");
        std::fs::write(&preferred, "#!/bin/sh\nread -r _\nprintf 8.6.17\n").unwrap();
        std::fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o755)).unwrap();

        let selection = locate_tclsh_in_with_prefixes(
            None,
            Some(TclAbi::Tcl86),
            Some(path_dir.path().as_os_str()),
            &[(Some(TclAbi::Tcl86), prefix.path().to_path_buf())],
        )
        .unwrap();
        assert_eq!(selection.abi, TclAbi::Tcl86);
        assert_eq!(selection.path, preferred);
        assert!(selection.warning.is_none());
    }
}
