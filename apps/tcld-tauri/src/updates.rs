use std::sync::atomic::{AtomicBool, Ordering};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

static CHECKING: AtomicBool = AtomicBool::new(false);

/// Updates happen only after a menu request and explicit installation consent.
pub fn check(app: tauri::AppHandle) {
    if CHECKING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = check_and_install(&app).await;
        CHECKING.store(false, Ordering::Release);
        if let Err(error) = result {
            app.dialog()
                .message(format!("Could not update TCLD.\n\n{error}"))
                .title("TCLD Updates")
                .show(|_| {});
        }
    });
}

async fn check_and_install(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        app.dialog().message("Update TCLD using your package manager. In-app installation is available for AppImage distributions.")
            .title("TCLD Updates").show(|_| {});
        return Ok(());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(update) = update else {
        app.dialog()
            .message("TCLD is up to date.")
            .title("TCLD Updates")
            .show(|_| {});
        return Ok(());
    };
    let dialog_app = app.clone();
    let version = update.version.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        dialog_app.dialog()
            .message(format!("TCLD {version} is available. Install it and restart? Save your work before continuing."))
            .title("TCLD Updates")
            .buttons(MessageDialogButtons::OkCancelCustom("Install and Restart".into(), "Cancel".into()))
            .blocking_show()
    }).await.map_err(|e| e.to_string())?;
    if accepted {
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        app.restart();
    }
    Ok(())
}

/// Exercise the real verifier/installer against a loopback HTTPS test server.
/// This is explicit CLI diagnostics, never an alternate production update feed.
pub fn distribution_probe(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let result = probe(&app).await;
        let code = match result {
            Ok(message) => {
                println!("TCLD updater probe: {message}");
                0
            }
            Err(error) => {
                eprintln!("TCLD updater probe failed: {error}");
                1
            }
        };
        app.exit(code);
    });
}

async fn probe(app: &tauri::AppHandle) -> Result<&'static str, String> {
    let endpoint = std::env::var("TCLD_UPDATER_PROBE_ENDPOINT").map_err(|e| e.to_string())?;
    let url = tauri::Url::parse(&endpoint).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err("the updater probe requires loopback HTTPS".into());
    }
    let action = std::env::var("TCLD_UPDATER_PROBE_ACTION").unwrap_or_else(|_| "check".into());
    if !matches!(action.as_str(), "check" | "download" | "install") {
        return Err("invalid updater probe action".into());
    }
    let mut builder = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .configure_client(|client| client.danger_accept_invalid_certs(true));
    // An explicit package probe can reinstall its own version into a disposable
    // app copy. Normal menu checks retain the default SemVer comparison.
    if action != "check" {
        builder = builder.version_comparator(|_, _| true);
    }
    let update = builder
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Ok("up to date");
    };
    if action == "check" {
        return Ok("update available");
    }
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    if action == "install" {
        update.install(bytes).map_err(|e| e.to_string())?;
        return Ok("installed");
    }
    Ok("download signature verified")
}
