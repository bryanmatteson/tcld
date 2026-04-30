#[test]
fn launch_profile_window_contract_exposes_stable_command_and_event_names() {
    let contract = tcld_tauri::launch_profile_window_contract();

    assert_eq!(contract.command, "open_launch_profile_window");
    assert_eq!(contract.window_label, "launch-profiles");
    assert_eq!(contract.main_to_profile_events, ["launch-profile:edit"]);
    assert_eq!(
        contract.profile_to_main_events,
        [
            "launch-profile:save",
            "launch-profile:cancel",
            "launch-profile:delete"
        ]
    );
    assert!(
        !contract.debugger_commands_changed,
        "profile window creation must not change debugger execution semantics"
    );
}
