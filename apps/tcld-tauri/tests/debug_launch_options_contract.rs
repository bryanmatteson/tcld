//! Contract for the debugger launch-options wire shape consumed by the
//! GUI (`apps/tcld-gui/src/lib/tauri-api.ts::DebugLaunchOptions`):
//! camelCase field names, `backend` defaulting to `embedded`, and the
//! external-backend override fields.

#![cfg(feature = "debugger")]

use tcld_tauri::debugger::{DebugBackendKind, DebugLaunchOptions};

#[test]
fn launch_options_default_to_embedded_backend() {
    let options: DebugLaunchOptions = serde_json::from_str("{}").expect("empty options parse");
    assert_eq!(options.backend, DebugBackendKind::Embedded);
    assert!(options.tclsh_path.is_none());
    assert!(options.dylib_path.is_none());

    let options = DebugLaunchOptions::default();
    assert_eq!(options.backend, DebugBackendKind::Embedded);
}

#[test]
fn launch_options_parse_external_backend_with_overrides() {
    let options: DebugLaunchOptions = serde_json::from_str(
        r#"{
            "backend": "external",
            "tclshPath": "/opt/homebrew/bin/tclsh9.0",
            "dylibPath": "/tmp/libtcld_dbg.dylib",
            "cwd": "/tmp/work"
        }"#,
    )
    .expect("external options parse");
    assert_eq!(options.backend, DebugBackendKind::External);
    assert_eq!(
        options.tclsh_path.as_deref(),
        Some("/opt/homebrew/bin/tclsh9.0")
    );
    assert_eq!(
        options.dylib_path.as_deref(),
        Some("/tmp/libtcld_dbg.dylib")
    );
    assert_eq!(options.cwd.as_deref(), Some("/tmp/work"));
}

#[test]
fn launch_options_round_trip_backend_tokens() {
    for (kind, token) in [
        (DebugBackendKind::Embedded, "\"embedded\""),
        (DebugBackendKind::External, "\"external\""),
    ] {
        let serialized = serde_json::to_string(&kind).expect("serialize backend kind");
        assert_eq!(serialized, token);
        let parsed: DebugBackendKind =
            serde_json::from_str(&serialized).expect("parse backend kind");
        assert_eq!(parsed, kind);
    }

    let options = DebugLaunchOptions {
        backend: DebugBackendKind::External,
        ..Default::default()
    };
    let json = serde_json::to_value(&options).expect("serialize options");
    assert_eq!(
        json.get("backend").and_then(|b| b.as_str()),
        Some("external")
    );
    // camelCase field names are the GUI contract.
    assert!(json.get("tclshPath").is_some());
    assert!(json.get("dylibPath").is_some());
    assert!(json.get("bytecodeHookMode").is_some());
}

#[test]
fn unknown_backend_token_is_a_parse_error_not_a_silent_fallback() {
    let result = serde_json::from_str::<DebugLaunchOptions>(r#"{"backend": "quantum"}"#);
    assert!(result.is_err(), "unknown backend tokens must be rejected");
}
