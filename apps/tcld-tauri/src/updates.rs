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
