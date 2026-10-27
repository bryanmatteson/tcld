#[test]
fn preferences_window_contract_exposes_stable_route_command_and_events() {
    let contract = tcld_tauri::preferences_window_contract();

    assert_eq!(contract.command, "open_preferences_window");
    assert_eq!(contract.window_label, "preferences");
    assert_eq!(contract.title, "Preferences");
    assert_eq!(contract.route, "/?window=preferences");
    assert_eq!(
        contract.appearance_settings_load_command,
        "load_appearance_settings"
    );
    assert_eq!(
        contract.appearance_settings_save_command,
        "save_appearance_settings"
    );
    assert_eq!(
        contract.appearance_settings_missing_config_behavior,
        "null-for-browser-fallback"
    );
    assert!(
        contract
            .config_path
            .ends_with("tcld/appearance-settings.json")
    );
    assert!(
        !contract.debugger_commands_changed,
        "preferences appearance persistence must not alter debugger runtime semantics"
    );
}

#[test]
fn run_configuration_window_retains_existing_contract_while_matching_design_route() {
    let contract = tcld_tauri::run_configuration_window_contract();

    assert_eq!(contract.command, "open_run_configuration_window");
    assert_eq!(contract.window_label, "run-configuration");
    assert_eq!(contract.title, "Run Configuration");
    assert_eq!(contract.route, "/?window=run-configuration");
    assert_eq!(
        contract.main_to_configuration_events,
        ["run-configuration:edit"]
    );
    assert_eq!(
        contract.profile_to_main_events,
        [
            "run-configuration:save",
            "run-configuration:cancel",
            "run-configuration:delete",
            "run-configuration:import",
            "run-configuration:export"
        ]
    );
}

#[test]
fn native_menu_contract_wires_preferences_and_run_configuration_window_commands() {
    let contract = tcld_tauri::native_menu_run_configuration_contract();
    let items = contract.menu_items();

    let preferences = items
        .iter()
        .find(|item| item.frontend_action_id == "appearance:open")
        .expect("View menu routes to the preferences window action");
    assert_eq!(preferences.item_id, "appearance_open");
    assert_eq!(preferences.label, "Preferences...");
    assert_eq!(preferences.menu_path, ["View"]);

    assert_eq!(
        contract.preferences_window.command,
        "open_preferences_window"
    );
    assert_eq!(contract.preferences_window.route, "/?window=preferences");
    assert_eq!(
        contract.run_configuration_window.route,
        "/?window=run-configuration"
    );
}

#[test]
fn main_registers_preferences_and_appearance_settings_commands() {
    let main_rs = include_str!("../src/app.rs");

    assert!(main_rs.contains("open_preferences_window"));
    assert!(main_rs.contains("load_appearance_settings"));
    assert!(main_rs.contains("save_appearance_settings"));
    assert!(main_rs.contains("Result<Option<crate::AppearanceSettingsPayload>"));
    assert!(main_rs.contains("return Ok(None);"));
    assert!(main_rs.contains("appearance-settings.json"));
}

#[test]
fn design_parity_backend_contract_keeps_preferences_run_configuration_and_debugger_at_ipc_layer() {
    let preferences = tcld_tauri::preferences_window_contract();
    assert_eq!(preferences.appearance_changed_event, "appearance:changed");
    assert_eq!(
        preferences.preference_sections,
        [
            "Appearance",
            "Editor",
            "Keybindings",
            "Decompiler",
            "Debugger",
            "Run profiles",
            "Telemetry",
            "Advanced"
        ]
    );
    assert_eq!(
        preferences.appearance_controls,
        [
            "theme",
            "type-system",
            "density",
            "variable-sparklines",
            "disassembly-group-rails"
        ]
    );

    let run_configuration = tcld_tauri::run_configuration_window_contract();
    assert_eq!(
        run_configuration.design_window_behaviors,
        ["edit", "save", "cancel", "delete", "import", "export"]
    );

    let native = tcld_tauri::native_menu_run_configuration_contract();
    assert_eq!(
        native.preferences_window.appearance_changed_event,
        "appearance:changed"
    );
    assert!(
        native
            .menu_items()
            .iter()
            .any(|item| item.frontend_action_id == "debug:pause"
                && item.enablement == "debug-active"),
        "debug pause remains a frontend/Tauri IPC command, not a domain-crate concern"
    );
}
