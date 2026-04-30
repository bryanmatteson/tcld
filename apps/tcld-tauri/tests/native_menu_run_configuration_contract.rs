#[test]
fn native_menu_run_configuration_contract_covers_app_actions_and_window() {
    let contract = tcld_tauri::native_menu_run_configuration_contract();

    let menus: Vec<_> = contract.menus.iter().map(|menu| menu.label).collect();
    assert!(menus.contains(&"File"));
    assert!(menus.contains(&"View"));
    assert!(menus.contains(&"Navigate"));
    assert!(menus.contains(&"Run"));
    assert!(menus.contains(&"Debug"));
    assert!(menus.contains(&"Tools"));
    assert!(menus.contains(&"Help"));
    assert!(
        !menus.iter().any(|menu| matches!(
            *menu,
            "Layout" | "Source" | "Disassembly" | "Graph" | "Format" | "Appearance" | "Keybindings"
        )),
        "domain-specific command groups should not be top-level native menus"
    );

    let items = contract.menu_items();
    let action_ids: Vec<_> = items.iter().map(|item| item.frontend_action_id).collect();
    assert!(action_ids.contains(&"open"));
    assert!(action_ids.contains(&"panel:instructions"));
    assert!(action_ids.contains(&"panel:graph"));
    assert!(action_ids.contains(&"layout:default"));
    assert!(action_ids.contains(&"debug:launch-profile"));
    assert!(action_ids.contains(&"debug:run_to_cursor"));
    assert!(action_ids.contains(&"source:fold-all"));
    assert!(action_ids.contains(&"instructions:fold-all"));
    assert!(action_ids.contains(&"format:apply"));
    assert!(action_ids.contains(&"appearance:open"));
    assert!(action_ids.contains(&"keybindings:import"));
    assert!(action_ids.contains(&"palette:open"));

    let run_config = contract
        .menu_items()
        .into_iter()
        .find(|item| item.item_id == "run_configuration")
        .expect("Run > Run Configuration item is part of the native menu");
    assert_eq!(
        run_config.frontend_action_id,
        "run-configuration:open-window"
    );
    assert_eq!(run_config.menu_path, ["Run"]);

    assert_eq!(
        contract.run_configuration_window.command,
        "open_run_configuration_window"
    );
    assert_eq!(
        contract.run_configuration_window.window_label,
        "run-configuration"
    );
    assert_eq!(contract.run_configuration_window.title, "Run Configuration");
    assert_eq!(
        contract.run_configuration_window.route,
        "/?window=run-configuration"
    );
    assert_eq!(
        contract.run_configuration_window.profile_to_main_events,
        [
            "run-configuration:save",
            "run-configuration:cancel",
            "run-configuration:delete",
            "run-configuration:import",
            "run-configuration:export"
        ]
    );

    let recent = contract
        .recent_configurations_submenu
        .as_ref()
        .expect("Run menu exposes a recent configurations submenu contract");
    assert_eq!(recent.menu_path, ["Run", "Recent Configurations"]);
    assert_eq!(recent.item_id_prefix, "run_recent_configuration:");
    assert_eq!(
        recent.frontend_action_id_prefix,
        "run-configuration:recent:"
    );
}

#[test]
fn native_menu_items_are_tied_to_frontend_command_ids_and_enablement_groups() {
    let contract = tcld_tauri::native_menu_run_configuration_contract();
    let items = contract.menu_items();

    assert!(
        items.iter().all(|item| !item.frontend_action_id.is_empty()),
        "every native menu item must route through a frontend command id"
    );
    assert!(
        items.iter().any(|item| item.enablement == "active-file"),
        "active-file enablement group is represented"
    );
    assert!(
        items.iter().any(|item| item.enablement == "debug-paused"),
        "debug-paused enablement group is represented"
    );
    assert!(
        items.iter().any(|item| item.enablement == "selection"),
        "selection enablement group is represented"
    );
    assert!(
        items.iter().any(|item| item.enablement == "always"),
        "always-enabled commands are represented"
    );
}

#[test]
fn native_menu_contract_uses_conventional_app_menu_buckets() {
    let contract = tcld_tauri::native_menu_run_configuration_contract();
    let menus: Vec<_> = contract.menus.iter().map(|menu| menu.label).collect();

    assert_eq!(
        menus,
        ["File", "View", "Navigate", "Run", "Debug", "Tools", "Help"]
    );

    let preferences = contract
        .item_by_id("appearance_open")
        .expect("Preferences item remains present");
    assert_eq!(preferences.label, "Preferences...");
    assert_eq!(preferences.accelerator, Some("CmdOrCtrl+,"));
    assert_eq!(preferences.menu_path, ["View"]);

    let layout = contract
        .item_by_id("layout_default")
        .expect("Layout presets move under View");
    assert_eq!(layout.menu_path, ["View"]);

    for item_id in [
        "source_fold_all",
        "instructions_fold_all",
        "graph_fit",
        "format_apply",
        "keybindings_import",
    ] {
        let item = contract
            .item_by_id(item_id)
            .expect("tool command remains present");
        assert_eq!(item.menu_path, ["Tools"]);
    }
}

#[test]
fn native_menu_run_configuration_contract_recent_configurations_menu_has_update_channel_and_real_slots()
 {
    let main_rs = include_str!("../src/main.rs");

    assert!(
        main_rs.contains("menu:recent-configurations"),
        "Tauri must listen for GUI-provided recent configuration menu contents"
    );
    assert!(
        main_rs.contains("RECENT_CONFIGURATION_MENU_SLOTS"),
        "Run > Recent Configurations must be backed by real menu slots"
    );
    assert!(
        main_rs.contains("set_text(item.label)") && main_rs.contains("set_enabled(item.enabled)"),
        "recent configuration slots must be relabeled and enabled from production payloads"
    );
    assert!(
        !main_rs.contains("run_recent_configuration:empty"),
        "recent configurations submenu cannot remain a disabled placeholder only"
    );
}
