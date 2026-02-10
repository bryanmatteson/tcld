use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug)]
enum TclAbi {
    Tcl85,
    Tcl86,
    Tcl90,
}

impl TclAbi {
    fn version_prefix(self) -> &'static str {
        match self {
            Self::Tcl85 => "8.5",
            Self::Tcl86 => "8.6",
            Self::Tcl90 => "9.",
        }
    }

    fn version_display(self) -> &'static str {
        match self {
            Self::Tcl85 => "8.5",
            Self::Tcl86 => "8.6",
            Self::Tcl90 => "9.0",
        }
    }

    fn dylib_name(self) -> &'static str {
        match self {
            Self::Tcl85 => "tcl8.5",
            Self::Tcl86 => "tcl8.6",
            Self::Tcl90 => "tcl9.0",
        }
    }

    fn stub_name(self) -> &'static str {
        match self {
            Self::Tcl85 => "tclstub8.5",
            Self::Tcl86 => "tclstub8.6",
            Self::Tcl90 => "tclstub9.0",
        }
    }

    fn version_env_key(self) -> &'static str {
        match self {
            Self::Tcl85 => "TCL8_5_PREFIX",
            Self::Tcl86 => "TCL8_6_PREFIX",
            Self::Tcl90 => "TCL9_PREFIX",
        }
    }

    fn tclsh_candidates(self) -> &'static [&'static str] {
        match self {
            Self::Tcl85 => &["tclsh8.5", "tclsh"],
            Self::Tcl86 => &["tclsh8.6", "tclsh"],
            Self::Tcl90 => &["tclsh9.0", "tclsh9", "tclsh"],
        }
    }

    fn framework_version(self) -> Option<&'static str> {
        match self {
            Self::Tcl85 => Some("8.5"),
            Self::Tcl86 => Some("8.6"),
            Self::Tcl90 => Some("9.0"),
        }
    }
}

fn main() {
    let abi = selected_abi();
    let loadable_extension = std::env::var_os("CARGO_FEATURE_LOADABLE_EXTENSION").is_some();
    let prefix = find_tcl_prefix(abi, !loadable_extension);
    let include_dir = resolve_include_dir(&prefix, abi)
        .unwrap_or_else(|| panic!("Tcl include directory not found under {prefix}"));
    println!("cargo:include={include_dir}");
    println!("cargo:active_version={}", abi.version_display());
    println!("cargo:stub_lib={}", abi.stub_name());
    if loadable_extension {
        println!("cargo:loadable_extension=1");
    } else {
        let lib_dir = resolve_lib_dir(&prefix, abi)
            .unwrap_or_else(|| panic!("Tcl library directory not found under {prefix}"));
        println!("cargo:rustc-link-search=native={lib_dir}");
        println!("cargo:rustc-link-lib=dylib={}", abi.dylib_name());
        println!("cargo:lib_dir={lib_dir}");
        println!("cargo:link_lib={}", abi.dylib_name());
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
        }
    }
    println!("cargo:rerun-if-env-changed=TCL_PREFIX");
    println!("cargo:rerun-if-env-changed={}", abi.version_env_key());
}

fn selected_abi() -> TclAbi {
    let tcl85 = std::env::var_os("CARGO_FEATURE_TCL8_5").is_some();
    let tcl86 = std::env::var_os("CARGO_FEATURE_TCL8_6").is_some();
    let tcl9 = std::env::var_os("CARGO_FEATURE_TCL9").is_some();

    match (tcl85, tcl86, tcl9) {
        (true, false, false) => TclAbi::Tcl85,
        (false, true, false) => TclAbi::Tcl86,
        (false, false, true) => TclAbi::Tcl90,
        (false, false, false) => {
            panic!("tcl-sys requires exactly one feature: enable `tcl8_5`, `tcl8_6`, or `tcl9`")
        }
        _ => {
            panic!("tcl-sys features `tcl8_5`, `tcl8_6`, and `tcl9` are mutually exclusive")
        }
    }
}

fn find_tcl_prefix(abi: TclAbi, require_lib: bool) -> String {
    if let Ok(prefix) = std::env::var(abi.version_env_key())
        && has_expected_layout(&prefix, abi, require_lib)
    {
        eprintln!("tcl-sys: using {}={prefix}", abi.version_env_key());
        return prefix;
    }

    if let Ok(prefix) = std::env::var("TCL_PREFIX")
        && has_expected_layout(&prefix, abi, require_lib)
    {
        eprintln!("tcl-sys: using TCL_PREFIX={prefix}");
        return prefix;
    }

    if let Some(prefix) = detect_via_tclsh(abi, require_lib) {
        eprintln!(
            "tcl-sys: detected Tcl {} prefix via tclsh: {prefix}",
            abi.version_display()
        );
        return prefix;
    }

    let mut candidates = vec![
        "/opt/homebrew".to_string(),
        "/usr/local".to_string(),
        "/usr".to_string(),
        "/opt/homebrew/opt/tcl-tk".to_string(),
        "/opt/homebrew/opt/tcl-tk@9".to_string(),
        "/opt/homebrew/opt/tcl-tk@8".to_string(),
        "/opt/local".to_string(),
    ];
    // macOS: Apple still ships Tcl 8.5 as the system shell; headers live in
    // the SDK framework (and sometimes /usr/include), not always next to
    // the runtime dylib under /System.
    if let Some(version) = abi.framework_version() {
        for sdk_root in macos_sdk_roots() {
            candidates.push(format!(
                "{sdk_root}/System/Library/Frameworks/Tcl.framework/Versions/{version}"
            ));
            candidates.push(format!(
                "{sdk_root}/Library/Frameworks/Tcl.framework/Versions/{version}"
            ));
        }
        candidates.push(format!(
            "/System/Library/Frameworks/Tcl.framework/Versions/{version}"
        ));
        candidates.push(format!(
            "/Library/Frameworks/Tcl.framework/Versions/{version}"
        ));
    }

    for candidate in &candidates {
        if has_expected_layout(candidate, abi, require_lib) {
            eprintln!(
                "tcl-sys: found Tcl {} at {candidate}",
                abi.version_display()
            );
            return candidate.clone();
        }
    }

    panic!(
        "Could not find Tcl {} installation.\n\
         Set {} or TCL_PREFIX to the Tcl install prefix,\n\
         e.g. {}=/opt/homebrew/opt/tcl-tk cargo build\n\
         Prefix must contain include/(tcl.h|tcl-tk/tcl.h|Headers/tcl.h){}",
        abi.version_display(),
        abi.version_env_key(),
        abi.version_env_key(),
        if require_lib {
            format!(" and lib/lib{}.{{dylib,so}}", abi.dylib_name())
        } else {
            String::new()
        }
    );
}

fn macos_sdk_roots() -> Vec<String> {
    let mut roots = Vec::new();
    if let Ok(output) = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            roots.push(path);
        }
    }
    for candidate in [
        "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk",
        "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk",
    ] {
        if Path::new(candidate).exists() {
            roots.push(candidate.to_string());
        }
    }
    roots
}

fn has_expected_layout(prefix: &str, abi: TclAbi, require_lib: bool) -> bool {
    resolve_include_dir(prefix, abi).is_some()
        && (!require_lib || resolve_lib_dir(prefix, abi).is_some())
}

fn resolve_include_dir(prefix: &str, abi: TclAbi) -> Option<String> {
    // Homebrew/source installs: include/tcl.h or include/tcl-tk/tcl.h.
    // Debian/Ubuntu (tcl8.6-dev): include/tcl8.6/tcl.h.
    // macOS Tcl.framework: Headers/tcl.h next to the framework version root.
    let mut candidates = vec![
        format!("{prefix}/include"),
        format!("{prefix}/include/tcl-tk"),
        format!("{prefix}/include/{}", abi.dylib_name()),
        format!("{prefix}/Headers"),
        format!("{prefix}/../Headers"),
    ];
    if let Some(version) = abi.framework_version() {
        candidates.push(format!("{prefix}/Versions/{version}/Headers"));
    }
    candidates
        .into_iter()
        .find(|dir| Path::new(&format!("{dir}/tcl.h")).exists())
        .filter(|dir| header_matches_abi(dir, abi))
}

fn header_matches_abi(include_dir: &str, abi: TclAbi) -> bool {
    let header = format!("{include_dir}/tcl.h");
    let Ok(contents) = std::fs::read_to_string(&header) else {
        return false;
    };
    // Prefer explicit TCL_VERSION string when present; fall back to major/minor.
    if let Some(version) = define_string_value(&contents, "TCL_VERSION") {
        return version.starts_with(abi.version_prefix().trim_end_matches('.'))
            || match abi {
                TclAbi::Tcl90 => version.starts_with('9'),
                TclAbi::Tcl86 => version.starts_with("8.6"),
                TclAbi::Tcl85 => version.starts_with("8.5"),
            };
    }
    let major = define_int_value(&contents, "TCL_MAJOR_VERSION");
    let minor = define_int_value(&contents, "TCL_MINOR_VERSION");
    matches!(
        (abi, major, minor),
        (TclAbi::Tcl90, Some(9), _)
            | (TclAbi::Tcl86, Some(8), Some(6))
            | (TclAbi::Tcl85, Some(8), Some(5))
    )
}

fn define_rest_after_name(line: &str, name: &str) -> Option<String> {
    // Tcl headers vary in spacing: both `#define TCL_VERSION "9.0"` and the
    // packed Homebrew form `#define TCL_VERSION"9.0"` / `TCL_MAJOR_VERSION9`.
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let after_hash = trimmed.trim_start_matches('#').trim_start();
    let after_define = after_hash.strip_prefix("define")?.trim_start();
    let rest = after_define.strip_prefix(name)?;
    // Require a non-identifier boundary so `TCL_VERSION` does not match
    // `TCL_VERSION_FOO`.
    if rest
        .chars()
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(rest.trim_start().to_string())
}

fn define_string_value(contents: &str, name: &str) -> Option<String> {
    for line in contents.lines() {
        let Some(rest) = define_rest_after_name(line, name) else {
            continue;
        };
        if let Some(quoted) = rest.strip_prefix('"')
            && let Some(end) = quoted.find('"')
        {
            return Some(quoted[..end].to_string());
        }
    }
    None
}

fn define_int_value(contents: &str, name: &str) -> Option<u32> {
    for line in contents.lines() {
        let Some(rest) = define_rest_after_name(line, name) else {
            continue;
        };
        let token = rest
            .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '(')
            .next()
            .unwrap_or("");
        if let Ok(value) = token.parse() {
            return Some(value);
        }
    }
    None
}

fn resolve_lib_dir(prefix: &str, abi: TclAbi) -> Option<String> {
    // Homebrew/source installs: lib/. Debian/Ubuntu multiarch:
    // lib/<triple>-linux-gnu/. Framework installs may ship only a tbd/stub.
    let candidates = [
        format!("{prefix}/lib"),
        format!("{prefix}/lib/x86_64-linux-gnu"),
        format!("{prefix}/lib/aarch64-linux-gnu"),
        prefix.to_string(),
    ];
    candidates.into_iter().find(|dir| {
        let name = abi.dylib_name();
        Path::new(&format!("{dir}/lib{name}.dylib")).exists()
            || Path::new(&format!("{dir}/lib{name}.so")).exists()
            || Path::new(&format!("{dir}/{name}.lib")).exists()
            || Path::new(&format!("{dir}/lib{name}.tbd")).exists()
            || Path::new(&format!("{dir}/Tcl")).exists()
            || Path::new(&format!("{dir}/Tcl.tbd")).exists()
    })
}

fn detect_via_tclsh(abi: TclAbi, require_lib: bool) -> Option<String> {
    for bin in abi.tclsh_candidates() {
        let command = format!(
            "echo 'puts [info patchlevel]; puts [file dirname [file dirname [info library]]]' | {bin}"
        );
        let output = Command::new("sh").arg("-c").arg(&command).output().ok()?;
        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let mut lines = stdout.lines();
        let patchlevel = lines.next()?.trim().to_string();
        let prefix = lines.next()?.trim().to_string();
        if patchlevel.starts_with(abi.version_prefix())
            && !prefix.is_empty()
            && has_expected_layout(&prefix, abi, require_lib)
        {
            return Some(prefix);
        }

        // Framework-style layout: dirname twice yields the version root
        // (…/Tcl.framework/Versions/8.5). Headers may still only live in
        // the matching SDK; try that when runtime prefix lacks headers.
        if patchlevel.starts_with(abi.version_prefix())
            && !prefix.is_empty()
            && !require_lib
            && let Some(version) = abi.framework_version()
        {
            for sdk_root in macos_sdk_roots() {
                let sdk_prefix = format!(
                    "{sdk_root}/System/Library/Frameworks/Tcl.framework/Versions/{version}"
                );
                if has_expected_layout(&sdk_prefix, abi, false) {
                    return Some(sdk_prefix);
                }
            }
        }
    }

    None
}
