#[cfg(feature = "debugger")]
use crate::debugger;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Listener, State};
use tokio::sync::RwLock;

use tauri_plugin_log::{Builder as LogBuilder, Target, TargetKind};
use tcld_decompile::{
    DecompileOptions, DisassembleOptions, decompile_path_with_options,
    disassemble_path_with_options,
};
use tcld_syntax::{FoldingRange, FormatEdit, FormatOptions, SyntaxToken};

const RECENT_CONFIGURATION_MENU_SLOTS: usize = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentConfigurationMenuPayload {
    items: Vec<RecentConfigurationMenuPayloadItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentConfigurationMenuPayloadItem {
    item_id: String,
    label: String,
    enabled: bool,
}

// ─── DTOs (serializable for frontend) ───────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InstructionDetailDTO {
    instruction_index: usize,
    opcode: String,
    pc: i32,
    size: i32,
    argc: i32,
    operands: Vec<i32>,
    command_group: String,
    command_pc_range: String,
    jumps: JumpsDTO,
    jump_target: String,
    fallthrough: String,
    value: Option<String>,
}

#[derive(Serialize, Clone)]
struct JumpsDTO {
    conditional: bool,
    unconditional: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SyntaxTokenDTO {
    capture: String,
    start_byte: usize,
    end_byte: usize,
    start_row: usize,
    start_column: usize,
    end_row: usize,
    end_column: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FoldingRangeDTO {
    kind: String,
    start_byte: usize,
    end_byte: usize,
    start_row: usize,
    start_column: usize,
    end_row: usize,
    end_column: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FormatEditDTO {
    start_byte: usize,
    end_byte: usize,
    replacement: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormatRequestDTO {
    source: String,
    trim_trailing_whitespace: Option<bool>,
    collapse_blank_lines: Option<bool>,
    ensure_trailing_newline: Option<bool>,
}

// ─── State ──────────────────────────────────────────────────────────

struct AppState {
    loaded: Option<crate::LoadResult>,
}

/// Path handed over by the launcher (`tcld ui <file>` or `tcld-tauri <file>`).
/// The main window takes it once on mount and opens it like a recent file.
struct LaunchOpenPath(Mutex<Option<String>>);

// ─── Tauri Commands ─────────────────────────────────────────────────

/// The frontend consumes `debugger_available` to gate debug UI instead of
/// discovering missing commands via IPC errors. New capability fields are
/// additive; absent fields mean "unavailable".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCapabilities {
    pub debugger_available: bool,
}

#[tauri::command]
fn get_app_capabilities() -> AppCapabilities {
    AppCapabilities {
        debugger_available: cfg!(feature = "debugger"),
    }
}

/// Returns the launcher-supplied path once; later calls (reloads, other
/// windows) see `None` so the file is not reopened over the user's work.
#[tauri::command]
fn take_launch_open_path(state: State<'_, LaunchOpenPath>) -> Option<String> {
    state.0.lock().ok().and_then(|mut path| path.take())
}

#[tauri::command]
async fn load_file(
    path: String,
    state: State<'_, RwLock<AppState>>,
) -> Result<crate::LoadResult, String> {
    let path_clone = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || crate::do_load(&path_clone))
        .await
        .map_err(|e| e.to_string())??;
    let mut app = state.write().await;
    app.loaded = Some(result.clone());
    Ok(result)
}

/// Load a Tcl source snippet directly from text. Used for the "Browse
/// examples" picker on the welcome screen so curated examples can be loaded
/// without writing a temp file or shipping an extra fs plugin.
#[tauri::command]
async fn load_text(
    source: String,
    label: String,
    state: State<'_, RwLock<AppState>>,
) -> Result<crate::LoadResult, String> {
    let source_clone = source.clone();
    let label_clone = label.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::do_load_source(&source_clone, &label_clone)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut app = state.write().await;
    app.loaded = Some(result.clone());
    Ok(result)
}

#[tauri::command]
async fn load_example(
    id: String,
    state: State<'_, RwLock<AppState>>,
) -> Result<crate::LoadResult, String> {
    let result = tauri::async_runtime::spawn_blocking(move || crate::load_bundled_example(&id))
        .await
        .map_err(|error| error.to_string())??;
    state.write().await.loaded = Some(result.clone());
    Ok(result)
}

#[tauri::command]
async fn get_instruction_detail(
    index: usize,
    state: State<'_, RwLock<AppState>>,
) -> Result<InstructionDetailDTO, String> {
    let app = state.read().await;
    let loaded = app.loaded.as_ref().ok_or("no file loaded")?;
    let inst = loaded
        .instructions
        .get(index)
        .ok_or("instruction index out of range")?;

    // Find command group details
    let command_group = inst
        .group
        .map(|g| format!("c{g:03}"))
        .unwrap_or_else(|| "n/a".to_string());

    let command_pc_range = inst
        .command_pc_range
        .map(|range| format!("{}..{}", range[0], range[1]))
        .unwrap_or_else(|| "n/a".to_string());

    let jump_target = if inst.is_jump {
        inst.jump_target
            .map(|t| t.to_string())
            .unwrap_or_else(|| "n/a".to_string())
    } else {
        "n/a".to_string()
    };

    let fallthrough = inst
        .fallthrough
        .map(|pc| pc.to_string())
        .unwrap_or_else(|| "n/a".to_string());

    let value = if inst.comment.is_empty() || inst.is_jump {
        None
    } else {
        Some(inst.comment.clone())
    };

    // Find trace info
    let trace = loaded.traces.get(index);

    let mut detail_value = value;
    if let Some(tf) = trace {
        let mut extra = String::new();
        if !tf.popped.is_empty() {
            extra.push_str(&format!("\npopped: [{}]", tf.popped.join(", ")));
        }
        if !tf.pushed.is_empty() {
            extra.push_str(&format!("\npushed: [{}]", tf.pushed.join(", ")));
        }
        extra.push_str(&format!(
            "\nstack: {} → {} (Δ{})",
            tf.before_depth, tf.after_depth, tf.stack_delta
        ));
        detail_value = Some(detail_value.map(|v| format!("{v}{extra}")).unwrap_or(extra));
    }

    Ok(InstructionDetailDTO {
        instruction_index: index,
        opcode: inst.op.clone(),
        pc: inst.pc,
        size: inst.size,
        argc: inst.argc,
        operands: inst.operands.clone(),
        command_group,
        command_pc_range,
        jumps: JumpsDTO {
            conditional: inst.is_jump_conditional,
            unconditional: inst.is_jump_unconditional,
        },
        jump_target,
        fallthrough,
        value: detail_value,
    })
}

#[tauri::command]
async fn get_decompile_full(state: State<'_, RwLock<AppState>>) -> Result<String, String> {
    let app = state.read().await;
    let loaded = app.loaded.as_ref().ok_or("no file loaded")?;
    Ok(loaded.decompile_text.clone())
}

#[tauri::command]
async fn disassemble_file(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        disassemble_path_with_options(
            &path,
            DisassembleOptions {
                text_truncate: Some(90),
                line_width: None,
            },
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn decompile_file(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        decompile_path_with_options(&path, DecompileOptions).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn read_text_file(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_text_file(path: String, contents: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, contents).map_err(|e| format!("write {}: {e}", path))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn highlight_tcl_source(source: String) -> Result<Vec<SyntaxTokenDTO>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let tokens: Vec<SyntaxToken> =
            tcld_syntax::highlight_tcl(&source).map_err(|e| e.to_string())?;
        Ok(tokens
            .into_iter()
            .map(|token| SyntaxTokenDTO {
                capture: token.capture,
                start_byte: token.start_byte,
                end_byte: token.end_byte,
                start_row: token.start_row,
                start_column: token.start_column,
                end_row: token.end_row,
                end_column: token.end_column,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn fold_tcl_source(source: String) -> Result<Vec<FoldingRangeDTO>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ranges: Vec<FoldingRange> =
            tcld_syntax::folding_ranges(&source).map_err(|e| e.to_string())?;
        Ok(ranges
            .into_iter()
            .map(|range| FoldingRangeDTO {
                kind: crate::folding_range_kind_label(range.kind).to_string(),
                start_byte: range.start_byte,
                end_byte: range.end_byte,
                start_row: range.start_row,
                start_column: range.start_column,
                end_row: range.end_row,
                end_column: range.end_column,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn format_tcl_source(request: FormatRequestDTO) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let options = FormatOptions {
            trim_trailing_whitespace: request.trim_trailing_whitespace.unwrap_or(true),
            collapse_blank_lines: request.collapse_blank_lines.unwrap_or(false),
            ensure_trailing_newline: request.ensure_trailing_newline.unwrap_or(true),
        };
        tcld_syntax::format_tcl_with_options(&request.source, options).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn plan_tcl_format_edits(request: FormatRequestDTO) -> Result<Vec<FormatEditDTO>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let options = FormatOptions {
            trim_trailing_whitespace: request.trim_trailing_whitespace.unwrap_or(true),
            collapse_blank_lines: request.collapse_blank_lines.unwrap_or(false),
            ensure_trailing_newline: request.ensure_trailing_newline.unwrap_or(true),
        };
        let edits: Vec<FormatEdit> =
            tcld_syntax::plan_format_edits(&request.source, options).map_err(|e| e.to_string())?;
        Ok(edits
            .into_iter()
            .map(|edit| FormatEditDTO {
                start_byte: edit.start_byte,
                end_byte: edit.end_byte,
                replacement: edit.replacement,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_launch_profile_window(app: tauri::AppHandle) -> Result<(), String> {
    let contract = crate::launch_profile_window_contract();
    if let Some(window) = app.get_webview_window(contract.window_label) {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        contract.window_label,
        tauri::WebviewUrl::App("/?window=launch-profiles".into()),
    )
    .title("Launch Profiles")
    .inner_size(720.0, 520.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn open_run_configuration_window(app: tauri::AppHandle) -> Result<(), String> {
    let contract = crate::run_configuration_window_contract();
    if let Some(window) = app.get_webview_window(contract.window_label) {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        contract.window_label,
        tauri::WebviewUrl::App(contract.route.into()),
    )
    .title(contract.title)
    .inner_size(760.0, 560.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn open_preferences_window(app: tauri::AppHandle) -> Result<(), String> {
    let contract = crate::preferences_window_contract();
    if let Some(window) = app.get_webview_window(contract.window_label) {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        contract.window_label,
        tauri::WebviewUrl::App(contract.route.into()),
    )
    .title(contract.title)
    .inner_size(1100.0, 780.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn appearance_settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("tcld");
    Ok(dir.join("appearance-settings.json"))
}

#[tauri::command]
async fn load_appearance_settings(
    app: tauri::AppHandle,
) -> Result<Option<crate::AppearanceSettingsPayload>, String> {
    let path = appearance_settings_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_appearance_settings(
    app: tauri::AppHandle,
    settings: crate::AppearanceSettingsPayload,
) -> Result<(), String> {
    let path = appearance_settings_path(&app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

struct DebugMenuItems {
    all: HashMap<String, tauri::menu::MenuItem<tauri::Wry>>,
    recent_configurations: HashMap<String, tauri::menu::MenuItem<tauri::Wry>>,
    debug_continue: tauri::menu::MenuItem<tauri::Wry>,
    debug_stop: tauri::menu::MenuItem<tauri::Wry>,
    debug_step_into: tauri::menu::MenuItem<tauri::Wry>,
    debug_step_over: tauri::menu::MenuItem<tauri::Wry>,
    debug_step_out: tauri::menu::MenuItem<tauri::Wry>,
    debug_toggle_breakpoint: tauri::menu::MenuItem<tauri::Wry>,
    debug_clear_all_breakpoints: tauri::menu::MenuItem<tauri::Wry>,
}

fn native_menu_has_separator_before(item_id: &str) -> bool {
    matches!(
        item_id,
        "file_copy_panel"
            | "file_export_decompile"
            | "layout_default"
            | "appearance_open"
            | "nav_back"
            | "nav_cycle_focus"
            | "debug_step_into"
            | "debug_toggle_breakpoint"
            | "source_fold_all"
            | "view_graph"
            | "format_preview"
            | "keybindings_import"
    )
}

fn build_menu(
    app: &tauri::App,
) -> Result<(tauri::menu::Menu<tauri::Wry>, DebugMenuItems), Box<dyn std::error::Error>> {
    let contract = crate::native_menu_run_configuration_contract();
    let mut menu_items: HashMap<String, tauri::menu::MenuItem<tauri::Wry>> = HashMap::new();
    let mut recent_configuration_items: HashMap<String, tauri::menu::MenuItem<tauri::Wry>> =
        HashMap::new();
    let mut submenus = Vec::new();

    for menu_contract in &contract.menus {
        let mut submenu = SubmenuBuilder::new(app, menu_contract.label);
        for item_contract in &menu_contract.items {
            if native_menu_has_separator_before(item_contract.item_id) {
                submenu = submenu.separator();
            }
            if menu_contract.label == "Run" && item_contract.item_id == "run_configuration" {
                let recent = contract
                    .recent_configurations_submenu
                    .as_ref()
                    .expect("recent configuration submenu contract");
                let mut recent_submenu = SubmenuBuilder::new(app, "Recent Configurations");
                for slot in 0..RECENT_CONFIGURATION_MENU_SLOTS {
                    let item_id = format!("{}{}", recent.item_id_prefix, slot);
                    let recent_item =
                        MenuItemBuilder::new(format!("Recent Configuration {}", slot + 1))
                            .id(item_id.as_str())
                            .enabled(false)
                            .build(app)?;
                    recent_configuration_items.insert(item_id, recent_item.clone());
                    recent_submenu = recent_submenu.item(&recent_item);
                }
                let recent_submenu = recent_submenu.build()?;
                submenu = submenu.separator().item(&recent_submenu).separator();
            }
            let mut builder = MenuItemBuilder::new(item_contract.label).id(item_contract.item_id);
            if let Some(accelerator) = item_contract.accelerator {
                builder = builder.accelerator(accelerator);
            }
            let menu_item = builder.build(app)?;
            if item_contract.enablement != "always" {
                let _ = menu_item.set_enabled(false);
            }
            menu_items.insert(item_contract.item_id.to_string(), menu_item.clone());
            submenu = submenu.item(&menu_item);
        }
        submenus.push((menu_contract.label, submenu.build()?));
    }

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let mut menu_builder = MenuBuilder::new(app);
    for label in [
        "File", "Edit", "View", "Navigate", "Run", "Debug", "Tools", "Window", "Help",
    ] {
        if label == "Edit" {
            menu_builder = menu_builder.item(&edit_menu);
        } else if label == "Window" {
            menu_builder = menu_builder.item(&window_menu);
        } else if let Some((_, submenu)) = submenus
            .iter()
            .find(|(submenu_label, _)| *submenu_label == label)
        {
            menu_builder = menu_builder.item(submenu);
        }
    }
    let menu = menu_builder.build()?;

    let items = DebugMenuItems {
        debug_continue: menu_items
            .get("debug_continue")
            .expect("debug_continue menu item")
            .clone(),
        debug_stop: menu_items
            .get("debug_stop")
            .expect("debug_stop menu item")
            .clone(),
        debug_step_into: menu_items
            .get("debug_step_into")
            .expect("debug_step_into menu item")
            .clone(),
        debug_step_over: menu_items
            .get("debug_step_over")
            .expect("debug_step_over menu item")
            .clone(),
        debug_step_out: menu_items
            .get("debug_step_out")
            .expect("debug_step_out menu item")
            .clone(),
        debug_toggle_breakpoint: menu_items
            .get("debug_toggle_breakpoint")
            .expect("debug_toggle_breakpoint menu item")
            .clone(),
        debug_clear_all_breakpoints: menu_items
            .get("debug_clear_all_breakpoints")
            .expect("debug_clear_all_breakpoints menu item")
            .clone(),
        all: menu_items,
        recent_configurations: recent_configuration_items,
    };

    Ok((menu, items))
}

#[cfg(feature = "debugger")]
fn setup_debugger(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let (debug_cmd_tx, debug_cmd_rx) = tokio::sync::mpsc::channel(64);
    app.manage(debug_cmd_tx);
    let app_handle_for_actor = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let actor = debugger::DebugActor::new(app_handle_for_actor);
        actor.run(debug_cmd_rx).await;
    });
    Ok(())
}

/// Startup options shared by the `tcld-tauri` binary and `tcld ui`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    /// File to open in the main window once the frontend is ready.
    pub open_path: Option<PathBuf>,
}

/// Run the desktop app on the calling thread until its last window closes.
///
/// Must be called from the process's main thread (a macOS/AppKit requirement).
pub fn run(options: LaunchOptions) -> Result<(), String> {
    // Build the async runtime before the log plugin installs its logger. The
    // plugin's webview target spawns onto this runtime from inside `log`, and
    // a lazily built runtime logs (mio registration traces when tokio's signal
    // driver is unified in, as under `tcld ui`) while its `OnceLock` is still
    // initializing — a re-entrant init that deadlocks before any window opens.
    let _ = tauri::async_runtime::handle();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::default()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        );

    let builder = builder
        .manage(RwLock::new(AppState { loaded: None }))
        .manage(LaunchOpenPath(Mutex::new(
            options
                .open_path
                .map(|path| path.to_string_lossy().into_owned()),
        )));

    let builder = builder.setup(|app| {
        #[cfg(feature = "debugger")]
        setup_debugger(app)?;

        let (menu, debug_items) = build_menu(app)?;
        app.set_menu(menu)?;

        app.on_menu_event(|app_handle, event| {
            let id = event.id().0.as_str();
            let contract = crate::native_menu_run_configuration_contract();
            if contract.item_by_id(id).is_some()
                || contract
                    .recent_configurations_submenu
                    .as_ref()
                    .is_some_and(|recent| id.starts_with(recent.item_id_prefix))
            {
                let _ = app_handle.emit(contract.menu_action_event, id);
            }
        });

        let items = debug_items;
        let all_menu_items = items.all.clone();
        let recent_configuration_items = items.recent_configurations.clone();
        app.listen("menu:availability", move |event| {
            let Ok(payload) = serde_json::from_str::<HashMap<String, bool>>(event.payload()) else {
                return;
            };
            for (item_id, enabled) in payload {
                if let Some(item) = all_menu_items.get(&item_id) {
                    let _ = item.set_enabled(enabled);
                }
            }
        });

        app.listen("menu:recent-configurations", move |event| {
            let Ok(payload) =
                serde_json::from_str::<RecentConfigurationMenuPayload>(event.payload())
            else {
                return;
            };
            for item in payload.items {
                if let Some(menu_item) = recent_configuration_items.get(&item.item_id) {
                    let _ = menu_item.set_text(item.label);
                    let _ = menu_item.set_enabled(item.enabled);
                }
            }
        });

        app.listen("debug:state-changed", move |event| {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct DebugStatePayload {
                session_state: String,
                has_bps: bool,
                #[allow(dead_code)]
                has_stopped_at: bool,
            }

            let Ok(payload) = serde_json::from_str::<DebugStatePayload>(event.payload()) else {
                return;
            };

            let is_paused = payload.session_state == "paused";
            let is_active = is_paused
                || payload.session_state == "running"
                || payload.session_state == "connected";

            let _ = items.debug_continue.set_enabled(is_paused);
            let _ = items.debug_stop.set_enabled(is_active);
            let _ = items.debug_step_into.set_enabled(is_paused);
            let _ = items.debug_step_over.set_enabled(is_paused);
            let _ = items.debug_step_out.set_enabled(is_paused);
            let _ = items
                .debug_clear_all_breakpoints
                .set_enabled(payload.has_bps);
            let _ = items.debug_toggle_breakpoint.set_enabled(is_active);
        });

        Ok(())
    });

    #[cfg(feature = "debugger")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_app_capabilities,
        take_launch_open_path,
        load_file,
        load_text,
        load_example,
        get_instruction_detail,
        get_decompile_full,
        disassemble_file,
        decompile_file,
        read_text_file,
        save_text_file,
        highlight_tcl_source,
        fold_tcl_source,
        format_tcl_source,
        plan_tcl_format_edits,
        open_launch_profile_window,
        open_run_configuration_window,
        open_preferences_window,
        load_appearance_settings,
        save_appearance_settings,
        debugger::debug_launch,
        debugger::debug_stop,
        debugger::debug_step_into,
        debugger::debug_step_instruction,
        debugger::debug_step_over_instruction,
        debugger::debug_step_out_instruction,
        debugger::debug_run_to_pc,
        debugger::debug_step_over,
        debugger::debug_step_out,
        debugger::debug_continue,
        debugger::debug_pause,
        debugger::debug_set_breakpoint,
        debugger::debug_set_breakpoint_enabled,
        debugger::debug_clear_breakpoint,
        debugger::debug_list_breakpoints,
        debugger::debug_eval,
        debugger::debug_get_variables,
        debugger::debug_get_stack,
        debugger::debug_set_variable,
        debugger::debug_get_state,
        debugger::debug_get_bytecode_chunks,
        debugger::debug_eval_sync,
        debugger::debug_disassemble,
        debugger::debug_resolve_command,
        debugger::debug_set_watchpoint,
        debugger::debug_clear_watchpoint,
        debugger::debug_list_watchpoints,
        debugger::debug_clear_all_watchpoints,
    ]);

    #[cfg(not(feature = "debugger"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_app_capabilities,
        take_launch_open_path,
        load_file,
        load_text,
        load_example,
        get_instruction_detail,
        get_decompile_full,
        disassemble_file,
        decompile_file,
        read_text_file,
        save_text_file,
        highlight_tcl_source,
        fold_tcl_source,
        format_tcl_source,
        plan_tcl_format_edits,
        open_launch_profile_window,
        open_run_configuration_window,
        open_preferences_window,
        load_appearance_settings,
        save_appearance_settings,
    ]);

    builder
        .run(tauri::generate_context!())
        .map_err(|error| format!("failed to run the tcld desktop app: {error}"))
}
