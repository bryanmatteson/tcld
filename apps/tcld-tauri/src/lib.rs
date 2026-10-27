use serde::{Deserialize, Serialize};

// The debugger lives in the library crate (not `main.rs`) so integration
// tests in `tests/` can drive the backends directly. The binary re-imports
// it; behavior is unchanged.
#[cfg(feature = "debugger")]
pub mod debugger;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProfileWindowContract {
    pub command: &'static str,
    pub window_label: &'static str,
    pub main_to_profile_events: [&'static str; 1],
    pub profile_to_main_events: [&'static str; 3],
    pub debugger_commands_changed: bool,
}

pub fn launch_profile_window_contract() -> LaunchProfileWindowContract {
    LaunchProfileWindowContract {
        command: "open_launch_profile_window",
        window_label: "launch-profiles",
        main_to_profile_events: ["launch-profile:edit"],
        profile_to_main_events: [
            "launch-profile:save",
            "launch-profile:cancel",
            "launch-profile:delete",
        ],
        debugger_commands_changed: false,
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesWindowContract {
    pub command: &'static str,
    pub window_label: &'static str,
    pub title: &'static str,
    pub route: &'static str,
    pub appearance_changed_event: &'static str,
    pub preference_sections: [&'static str; 8],
    pub appearance_controls: [&'static str; 5],
    pub appearance_settings_load_command: &'static str,
    pub appearance_settings_save_command: &'static str,
    pub appearance_settings_missing_config_behavior: &'static str,
    pub config_path: &'static str,
    pub debugger_commands_changed: bool,
}

pub fn preferences_window_contract() -> PreferencesWindowContract {
    PreferencesWindowContract {
        command: "open_preferences_window",
        window_label: "preferences",
        title: "Preferences",
        route: "/?window=preferences",
        appearance_changed_event: "appearance:changed",
        preference_sections: [
            "Appearance",
            "Editor",
            "Keybindings",
            "Decompiler",
            "Debugger",
            "Run profiles",
            "Telemetry",
            "Advanced",
        ],
        appearance_controls: [
            "theme",
            "type-system",
            "density",
            "variable-sparklines",
            "disassembly-group-rails",
        ],
        appearance_settings_load_command: "load_appearance_settings",
        appearance_settings_save_command: "save_appearance_settings",
        appearance_settings_missing_config_behavior: "null-for-browser-fallback",
        config_path: "tcld/appearance-settings.json",
        debugger_commands_changed: false,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettingsPayload {
    pub theme: String,
    pub type_system: String,
    pub density: String,
    pub show_sparklines: bool,
    pub group_rails: bool,
    pub font_size: u16,
}

impl Default for AppearanceSettingsPayload {
    fn default() -> Self {
        Self {
            theme: "graphite".to_string(),
            type_system: "mono-brutalist".to_string(),
            density: "regular".to_string(),
            show_sparklines: true,
            group_rails: true,
            font_size: 13,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunConfigurationWindowContract {
    pub command: &'static str,
    pub window_label: &'static str,
    pub title: &'static str,
    pub route: &'static str,
    pub design_window_behaviors: [&'static str; 6],
    pub main_to_configuration_events: [&'static str; 1],
    pub profile_to_main_events: [&'static str; 5],
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeMenuItemContract {
    pub item_id: &'static str,
    pub label: &'static str,
    pub frontend_action_id: &'static str,
    pub accelerator: Option<&'static str>,
    pub enablement: &'static str,
    pub menu_path: Vec<&'static str>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeMenuContract {
    pub label: &'static str,
    pub items: Vec<NativeMenuItemContract>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentConfigurationsSubmenuContract {
    pub menu_path: [&'static str; 2],
    pub item_id_prefix: &'static str,
    pub frontend_action_id_prefix: &'static str,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeMenuRunConfigurationContract {
    pub menus: Vec<NativeMenuContract>,
    pub preferences_window: PreferencesWindowContract,
    pub run_configuration_window: RunConfigurationWindowContract,
    pub recent_configurations_submenu: Option<RecentConfigurationsSubmenuContract>,
    pub menu_action_event: &'static str,
}

impl NativeMenuRunConfigurationContract {
    pub fn menu_items(&self) -> Vec<NativeMenuItemContract> {
        self.menus
            .iter()
            .flat_map(|menu| menu.items.iter().cloned())
            .collect()
    }

    pub fn item_by_id(&self, item_id: &str) -> Option<&NativeMenuItemContract> {
        self.menus
            .iter()
            .flat_map(|menu| menu.items.iter())
            .find(|item| item.item_id == item_id)
    }
}

pub fn run_configuration_window_contract() -> RunConfigurationWindowContract {
    RunConfigurationWindowContract {
        command: "open_run_configuration_window",
        window_label: "run-configuration",
        title: "Run Configuration",
        route: "/?window=run-configuration",
        design_window_behaviors: ["edit", "save", "cancel", "delete", "import", "export"],
        main_to_configuration_events: ["run-configuration:edit"],
        profile_to_main_events: [
            "run-configuration:save",
            "run-configuration:cancel",
            "run-configuration:delete",
            "run-configuration:import",
            "run-configuration:export",
        ],
    }
}

pub fn native_menu_run_configuration_contract() -> NativeMenuRunConfigurationContract {
    NativeMenuRunConfigurationContract {
        menu_action_event: "menu:action",
        preferences_window: preferences_window_contract(),
        run_configuration_window: run_configuration_window_contract(),
        recent_configurations_submenu: Some(RecentConfigurationsSubmenuContract {
            menu_path: ["Run", "Recent Configurations"],
            item_id_prefix: "run_recent_configuration:",
            frontend_action_id_prefix: "run-configuration:recent:",
        }),
        menus: vec![
            menu(
                "File",
                vec![
                    item(
                        "file_open",
                        "Open...",
                        "open",
                        Some("CmdOrCtrl+O"),
                        "always",
                        &["File"],
                    ),
                    item(
                        "file_close",
                        "Close Window",
                        "app:close-window",
                        None,
                        "always",
                        &["File"],
                    ),
                    item(
                        "file_copy_panel",
                        "Copy Active Panel",
                        "copy:panel",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_copy_diagnostics",
                        "Copy Diagnostics",
                        "copy:diagnostics",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_copy_full_report",
                        "Copy Full Report",
                        "copy:full_report",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_decompile",
                        "Export Decompiled Tcl...",
                        "export:decompile",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_disassembly",
                        "Export Disassembly...",
                        "export:disassembly",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_diagnostics",
                        "Export Diagnostics...",
                        "export:diagnostics",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_provenance",
                        "Export Provenance...",
                        "export:provenance",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_comparison",
                        "Export Comparison...",
                        "export:comparison",
                        None,
                        "active-file",
                        &["File"],
                    ),
                    item(
                        "file_export_console",
                        "Export Console...",
                        "export:console",
                        None,
                        "debug-active",
                        &["File"],
                    ),
                    item(
                        "file_export_full_report",
                        "Export Full Report...",
                        "export:full_report",
                        None,
                        "active-file",
                        &["File"],
                    ),
                ],
            ),
            menu(
                "View",
                vec![
                    item(
                        "view_instructions",
                        "Disassembly",
                        "panel:instructions",
                        Some("CmdOrCtrl+1"),
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_decompile",
                        "Decompile",
                        "panel:decompile",
                        Some("CmdOrCtrl+2"),
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_source",
                        "Source",
                        "panel:source",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_comparison",
                        "Compare",
                        "panel:comparison",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_diagnostics",
                        "Diagnostics",
                        "panel:diagnostics",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_provenance",
                        "Provenance",
                        "panel:provenance",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_project",
                        "Project",
                        "panel:project",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_detail",
                        "Detail",
                        "panel:detail",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_literals",
                        "Literals",
                        "panel:literals",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_exceptions",
                        "Exceptions",
                        "panel:exceptions",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_summary",
                        "Summary",
                        "panel:summary",
                        None,
                        "active-file",
                        &["View"],
                    ),
                    item(
                        "view_variables",
                        "Variables",
                        "panel:variables",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "view_stack",
                        "Call Stack",
                        "panel:stack",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "view_console",
                        "Console",
                        "panel:console",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "view_breakpoints",
                        "Breakpoints",
                        "panel:breakpoints",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "view_watchpoints",
                        "Watchpoints",
                        "panel:watchpoints",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "view_commandResolver",
                        "Command Resolver",
                        "panel:command_resolver",
                        None,
                        "debug-active",
                        &["View"],
                    ),
                    item(
                        "layout_default",
                        "Default Layout",
                        "layout:default",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "layout_debug",
                        "Debug Layout",
                        "layout:debug",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "layout_wide",
                        "Wide Layout",
                        "layout:wide",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "layout_analysis",
                        "Analysis Layout",
                        "layout:analysis",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "layout_decompile",
                        "Decompile Focus Layout",
                        "layout:decompile",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "layout_overview",
                        "Overview Layout",
                        "layout:overview",
                        None,
                        "always",
                        &["View"],
                    ),
                    item(
                        "appearance_open",
                        "Preferences...",
                        "appearance:open",
                        Some("CmdOrCtrl+,"),
                        "always",
                        &["View"],
                    ),
                    item(
                        "fontsize_up",
                        "Increase Font Size",
                        "fontsize:up",
                        Some("CmdOrCtrl+="),
                        "always",
                        &["View"],
                    ),
                    item(
                        "fontsize_down",
                        "Decrease Font Size",
                        "fontsize:down",
                        Some("CmdOrCtrl+-"),
                        "always",
                        &["View"],
                    ),
                    item(
                        "fontsize_reset",
                        "Reset Font Size",
                        "fontsize:reset",
                        None,
                        "always",
                        &["View"],
                    ),
                ],
            ),
            menu(
                "Navigate",
                vec![
                    item(
                        "nav_next",
                        "Next Instruction",
                        "nav:next",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_prev",
                        "Previous Instruction",
                        "nav:prev",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_top",
                        "Go To Top",
                        "nav:top",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_bottom",
                        "Go To Bottom",
                        "nav:bottom",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_next_group",
                        "Next Group",
                        "nav:next_group",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_prev_group",
                        "Previous Group",
                        "nav:prev_group",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_first_bookmark",
                        "First Bookmark",
                        "nav:first_bookmark",
                        None,
                        "selection",
                        &["Navigate"],
                    ),
                    item(
                        "nav_back",
                        "Back",
                        "nav:back",
                        Some("Alt+Left"),
                        "always",
                        &["Navigate"],
                    ),
                    item(
                        "nav_forward",
                        "Forward",
                        "nav:forward",
                        Some("Alt+Right"),
                        "always",
                        &["Navigate"],
                    ),
                    item(
                        "nav_cycle_focus",
                        "Cycle Focus",
                        "nav:cycle_focus",
                        None,
                        "always",
                        &["Navigate"],
                    ),
                    item(
                        "nav_cycle_focus_back",
                        "Cycle Focus Backward",
                        "nav:cycle_focus_back",
                        None,
                        "always",
                        &["Navigate"],
                    ),
                ],
            ),
            menu(
                "Run",
                vec![
                    item(
                        "debug_launch",
                        "Launch Script...",
                        "debug:launch",
                        Some("F5"),
                        "debug-launchable",
                        &["Run"],
                    ),
                    item(
                        "debug_launch_profile",
                        "Run Selected Configuration",
                        "debug:launch-profile",
                        None,
                        "debug-launchable",
                        &["Run"],
                    ),
                    item(
                        "run_configuration",
                        "Run Configuration...",
                        "run-configuration:open-window",
                        Some("CmdOrCtrl+Shift+R"),
                        "always",
                        &["Run"],
                    ),
                ],
            ),
            menu(
                "Debug",
                vec![
                    item(
                        "debug_continue",
                        "Continue",
                        "debug:continue",
                        Some("F5"),
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_pause",
                        "Pause",
                        "debug:pause",
                        Some("F6"),
                        "debug-active",
                        &["Debug"],
                    ),
                    item(
                        "debug_stop",
                        "Stop",
                        "debug:stop",
                        Some("Shift+F5"),
                        "debug-active",
                        &["Debug"],
                    ),
                    item(
                        "debug_restart",
                        "Restart",
                        "debug:restart",
                        Some("CmdOrCtrl+Shift+F5"),
                        "debug-active",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_into",
                        "Step Into",
                        "debug:step_into",
                        Some("F11"),
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_instruction",
                        "Step Instruction",
                        "debug:step_instruction",
                        None,
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_over_instruction",
                        "Step Over Instruction",
                        "debug:step_over_instruction",
                        None,
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_out_instruction",
                        "Step Out Instruction",
                        "debug:step_out_instruction",
                        None,
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_run_to_pc",
                        "Run To PC",
                        "debug:run_to_pc",
                        None,
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_next_group",
                        "Step To Next Group",
                        "debug:step_next_group",
                        None,
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_over",
                        "Step Over",
                        "debug:step_over",
                        Some("F10"),
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_step_out",
                        "Step Out",
                        "debug:step_out",
                        Some("Shift+F11"),
                        "debug-paused",
                        &["Debug"],
                    ),
                    item(
                        "debug_run_to_cursor",
                        "Run To Cursor",
                        "debug:run_to_cursor",
                        Some("Ctrl+F5"),
                        "debug-paused-selection",
                        &["Debug"],
                    ),
                    item(
                        "debug_toggle_breakpoint",
                        "Toggle Breakpoint",
                        "debug:toggle_breakpoint",
                        Some("F9"),
                        "selection",
                        &["Debug"],
                    ),
                    item(
                        "debug_clear_all_breakpoints",
                        "Clear All Breakpoints",
                        "debug:clear_breakpoints",
                        None,
                        "debug-active",
                        &["Debug"],
                    ),
                ],
            ),
            menu(
                "Tools",
                vec![
                    item(
                        "source_fold_all",
                        "Fold Source",
                        "source:fold-all",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "source_unfold_all",
                        "Unfold Source",
                        "source:unfold-all",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "instructions_fold_all",
                        "Fold Disassembly",
                        "instructions:fold-all",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "instructions_unfold_all",
                        "Unfold Disassembly",
                        "instructions:unfold-all",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "jumparrows",
                        "Toggle Jump Arrows",
                        "jumparrows",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "view_graph",
                        "Show Graph",
                        "panel:graph",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_fit",
                        "Fit Graph",
                        "graph:fit",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_center_selection",
                        "Center Selection",
                        "graph:center-selection",
                        None,
                        "selection",
                        &["Tools"],
                    ),
                    item(
                        "graph_copy_details",
                        "Copy Graph Details",
                        "graph:copy-details",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_expand_node",
                        "Expand Graph Node",
                        "graph:expand-node",
                        None,
                        "selection",
                        &["Tools"],
                    ),
                    item(
                        "graph_zoom_in",
                        "Zoom In",
                        "graph:zoom-in",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_zoom_out",
                        "Zoom Out",
                        "graph:zoom-out",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_mode_basic_block",
                        "Basic Block Graph Mode",
                        "graph:mode-basic-block",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "graph_mode_source_snippet",
                        "Source Snippet Graph Mode",
                        "graph:mode-source-snippet",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "format_preview",
                        "Preview Active Source",
                        "format:preview",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "format_apply",
                        "Apply Active Source",
                        "format:apply",
                        None,
                        "active-file",
                        &["Tools"],
                    ),
                    item(
                        "keybindings_import",
                        "Import Keybindings...",
                        "keybindings:import",
                        None,
                        "always",
                        &["Tools"],
                    ),
                    item(
                        "keybindings_export",
                        "Export Keybindings...",
                        "keybindings:export",
                        None,
                        "always",
                        &["Tools"],
                    ),
                    item(
                        "keybindings_reset",
                        "Reset Keybindings",
                        "keybindings:reset",
                        None,
                        "always",
                        &["Tools"],
                    ),
                ],
            ),
            menu(
                "Help",
                vec![
                    item(
                        "view_palette",
                        "Command Palette...",
                        "palette:open",
                        Some("CmdOrCtrl+P"),
                        "always",
                        &["Help"],
                    ),
                    item("help", "Help", "help", None, "always", &["Help"]),
                ],
            ),
        ],
    }
}

fn menu(label: &'static str, items: Vec<NativeMenuItemContract>) -> NativeMenuContract {
    NativeMenuContract { label, items }
}

fn item(
    item_id: &'static str,
    label: &'static str,
    frontend_action_id: &'static str,
    accelerator: Option<&'static str>,
    enablement: &'static str,
    menu_path: &[&'static str],
) -> NativeMenuItemContract {
    NativeMenuItemContract {
        item_id,
        label,
        frontend_action_id,
        accelerator,
        enablement,
        menu_path: menu_path.to_vec(),
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadataDTO {
    pub filename: String,
    pub path: String,
    pub tcl_version: String,
    pub instruction_count: usize,
    pub literal_count: usize,
    pub command_count: usize,
    pub exception_count: usize,
    pub aux_count: usize,
    pub code_bytes: i32,
    pub max_stack_depth: i32,
    pub syntax_engine: String,
    pub format: Option<i32>,
    pub build: Option<i32>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDTO {
    pub idx: usize,
    pub pc: i32,
    pub op: String,
    pub args: String,
    pub operands: Vec<i32>,
    pub argc: i32,
    pub group: Option<usize>,
    pub command_pc_range: Option<[i32; 2]>,
    pub group_start: bool,
    pub comment: String,
    pub size: i32,
    pub is_jump: bool,
    pub is_jump_conditional: bool,
    pub is_jump_unconditional: bool,
    pub jump_target: Option<i32>,
    pub fallthrough: Option<i32>,
    pub primary_next_location: Option<i32>,
    pub secondary_next_location: Option<i32>,
    pub next_locations: Vec<i32>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiteralDTO {
    pub idx: usize,
    pub kind: String,
    pub value: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionDTO {
    pub idx: usize,
    pub kind: String,
    pub nesting_level: i32,
    pub code_range: [i32; 2],
    pub break_offset: i32,
    pub continue_offset: i32,
    pub catch_offset: i32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BasicBlockDTO {
    pub id: usize,
    pub start: usize,
    pub end: usize,
    pub start_pc: i32,
    pub end_pc: i32,
    pub group: String,
    pub edges: Vec<usize>,
    pub instructions: Vec<BlockInstructionDTO>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockInstructionDTO {
    pub pc: i32,
    pub name: String,
    pub operands: Vec<i32>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TraceFrameDTO {
    pub instruction_index: usize,
    pub pc: i32,
    pub opcode: String,
    pub argc: i32,
    pub stack_delta: i32,
    pub before_depth: i32,
    pub after_depth: i32,
    pub popped: Vec<String>,
    pub pushed: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DecompileLineDTO {
    pub text: String,
    pub group: Option<usize>,
    pub line_index: usize,
    pub source_map_index: Option<usize>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxTokenDTO {
    pub capture: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRangeDTO {
    pub kind: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SourceMapEntryDTO {
    pub bytecode_range: [i32; 2],
    pub source_lines: [usize; 2],
    pub source_chars: [usize; 2],
    pub text: String,
    pub command_group: Option<usize>,
    pub command_path: Vec<usize>,
    pub parent_command_group: Option<usize>,
    pub original_status: String,
    pub original_span: Option<[i32; 2]>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SourceCommandAnchorDTO {
    pub kind: String,
    pub source_map_index: usize,
    pub command_path: Vec<usize>,
    pub display_path: String,
    pub parent_root_group: Option<usize>,
    pub command_group: Option<usize>,
    pub source_lines: [usize; 2],
    pub original_span: Option<[i32; 2]>,
    pub snippet: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticApproximationDTO {
    pub kind: String,
    pub bytecode_range: Option<[i32; 2]>,
    pub reason: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PassDecisionDTO {
    pub name: String,
    pub enabled: bool,
    pub iteration: usize,
    pub changed: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReportDTO {
    pub reconstruction: String,
    pub recognized_constructs: Vec<String>,
    pub unreduced_opcodes: Vec<String>,
    pub approximations: Vec<DiagnosticApproximationDTO>,
    pub suppressed_fast_path_ranges: Vec<[i32; 2]>,
    pub pass_decisions: Vec<PassDecisionDTO>,
    pub unsupported_instructions: Vec<String>,
    pub suggested_next_work: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoadProvenanceDTO {
    pub kind: String,
    pub label: String,
    pub original_source_available: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoadResult {
    pub metadata: FileMetadataDTO,
    pub instructions: Vec<InstructionDTO>,
    pub literals: Vec<LiteralDTO>,
    pub exceptions: Vec<ExceptionDTO>,
    pub blocks: Vec<BasicBlockDTO>,
    pub traces: Vec<TraceFrameDTO>,
    pub decompile_lines: Vec<DecompileLineDTO>,
    pub decompile_text: String,
    pub decompile_tokens: Vec<SyntaxTokenDTO>,
    pub decompile_folding_ranges: Vec<FoldingRangeDTO>,
    pub dump_text: String,
    pub summary_text: String,
    pub source_map: Vec<SourceMapEntryDTO>,
    pub source_command_anchors: Vec<SourceCommandAnchorDTO>,
    pub diagnostics: DiagnosticsReportDTO,
    pub reconstruction: String,
    pub provenance: LoadProvenanceDTO,
    pub original_source_text: Option<String>,
}

pub fn do_load(path: &str) -> Result<LoadResult, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "tcl" | "tk" | "itcl" | "tm")
        && tcld_syntax::extract_from_contents(&source).is_err()
    {
        let mut result = do_load_source(&source, path)?;
        result.provenance.kind = "Path".into();
        result.metadata.path = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.into())
            .display()
            .to_string();
        result.provenance.label = result.metadata.path.clone();
        result.metadata.filename = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .into();
        return Ok(result);
    }
    let snapshot =
        tcld_decompile::AnalysisSnapshot::from_input(tcld_decompile::LoadInput::path(path))
            .map_err(|e| e.to_string())?;
    Ok(snapshot_to_dto(&snapshot, "tree-sitter-tcl"))
}

/// Compile admitted source without evaluating it, then use the shared analysis
/// snapshot. Unsupported Tcl forms retain the compiler's precise diagnostic.
pub fn do_load_source(source: &str, label: &str) -> Result<LoadResult, String> {
    if tcld_syntax::extract_from_contents(source).is_ok() {
        let snapshot = tcld_decompile::AnalysisSnapshot::from_input(
            tcld_decompile::LoadInput::tcl_source_text(source, label),
        )
        .map_err(|error| error.to_string())?;
        return Ok(snapshot_to_dto(&snapshot, "tree-sitter-tcl"));
    }
    let image = tcld_compile::compile_source(
        label.into(),
        source.into(),
        tcld_compile::CompileMode::Hybrid,
    )
    .map_err(|error| error.to_string())?;
    let snapshot = tcld_decompile::AnalysisSnapshot::from_loaded(tcld_decompile::LoadedBytecode {
        bytecode: image.top_level().bytecode.clone(),
        provenance: tcld_decompile::LoadProvenance {
            kind: tcld_decompile::LoadKind::TclSourceText,
            label: label.into(),
            normalized_path: None,
            original_source_available: true,
            debugger_sequence: None,
        },
        payload_text: String::new(),
        original_source: Some(source.into()),
    })
    .map_err(|error| error.to_string())?;
    let mut result = snapshot_to_dto(&snapshot, "tree-sitter-tcl");
    result.metadata.tcl_version = "8.6".into();
    Ok(result)
}

/// Compile each bundled example from source through the same path as edits.
pub fn load_bundled_example(id: &str) -> Result<LoadResult, String> {
    let (source, label) = match id {
        "hello" => (
            include_str!("../../tcld/tests/fixtures/hello.tcl"),
            "hello.tcl",
        ),
        "proc" => (
            include_str!("../../tcld/tests/fixtures/proc.tcl"),
            "proc.tcl",
        ),
        "for-break" => (
            include_str!("../../tcld/tests/fixtures/break.tcl"),
            "break.tcl",
        ),
        "expr" => (
            include_str!("../../tcld/tests/fixtures/expr.tcl"),
            "expr.tcl",
        ),
        "foreach" => (
            include_str!("../../tcld/tests/fixtures/foreach.tcl"),
            "foreach.tcl",
        ),
        "catch" => (
            include_str!("../../tcld/tests/fixtures/catch.tcl"),
            "catch.tcl",
        ),
        _ => return Err(format!("Unknown bundled example: {id}")),
    };
    do_load_source(source, label)
}

pub fn do_load_from_payload(payload_text: &str, label: &str) -> Result<LoadResult, String> {
    let snapshot = tcld_decompile::AnalysisSnapshot::from_input(
        tcld_decompile::LoadInput::debugger_chunk(payload_text, label, 0),
    )
    .map_err(|e| e.to_string())?;
    Ok(snapshot_to_dto(&snapshot, "tree-sitter-tcl"))
}

pub fn load_file_snapshot(path: &str) -> Result<LoadResult, String> {
    do_load(path)
}

pub fn load_debugger_payload_snapshot(
    payload_text: &str,
    label: &str,
    _sequence: u64,
) -> Result<LoadResult, String> {
    do_load_from_payload(payload_text, label)
}

fn display_command_path(components: &[usize]) -> String {
    let Some((root, rest)) = components.split_first() else {
        return String::new();
    };
    let mut out = format!("c{root:03}");
    for nested in rest {
        out.push_str(&format!("/n{nested:03}"));
    }
    out
}

fn command_snippet(text: &str) -> String {
    let snippet = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    tcld_core::util::truncate(snippet, 90, "...")
}

pub fn folding_range_kind_label(kind: tcld_syntax::FoldingRangeKind) -> &'static str {
    match kind {
        tcld_syntax::FoldingRangeKind::Region => "region",
        tcld_syntax::FoldingRangeKind::Comment => "comment",
    }
}

pub fn folding_range_to_dto(range: tcld_syntax::FoldingRange) -> FoldingRangeDTO {
    FoldingRangeDTO {
        kind: folding_range_kind_label(range.kind).to_string(),
        start_byte: range.start_byte,
        end_byte: range.end_byte,
        start_row: range.start_row,
        start_column: range.start_column,
        end_row: range.end_row,
        end_column: range.end_column,
    }
}

fn snapshot_to_dto(snapshot: &tcld_decompile::AnalysisSnapshot, syntax_engine: &str) -> LoadResult {
    let source_map = snapshot
        .source_map
        .entries()
        .iter()
        .map(|entry| SourceMapEntryDTO {
            bytecode_range: [entry.bytecode_range.start, entry.bytecode_range.end],
            source_lines: [entry.source_lines.start, entry.source_lines.end],
            source_chars: [entry.source_chars.start, entry.source_chars.end],
            text: entry.text.clone(),
            command_group: entry.command_group(),
            command_path: entry.command_path.components.clone(),
            parent_command_group: entry.parent_command_group,
            original_status: match entry.original_status {
                tcld_decompile::OriginalSourceStatus::Available => "available".to_string(),
                tcld_decompile::OriginalSourceStatus::Unavailable => "unavailable".to_string(),
            },
            original_span: entry
                .original_span
                .as_ref()
                .map(|range| [range.start, range.end]),
        })
        .collect::<Vec<_>>();
    let source_command_anchors = snapshot
        .source_map
        .entries()
        .iter()
        .enumerate()
        .filter_map(|(source_map_index, entry)| {
            if entry.command_path.components.is_empty() {
                return None;
            }
            let kind = if entry.parent_command_group.is_some() {
                "nested"
            } else {
                "root"
            };
            let parent_root_group = entry
                .parent_command_group
                .or_else(|| entry.command_path.components.first().copied())
                .or_else(|| entry.command_group());
            Some(SourceCommandAnchorDTO {
                kind: kind.to_string(),
                source_map_index,
                command_path: entry.command_path.components.clone(),
                display_path: display_command_path(&entry.command_path.components),
                parent_root_group,
                command_group: entry.command_group(),
                source_lines: [entry.source_lines.start, entry.source_lines.end],
                original_span: entry
                    .original_span
                    .as_ref()
                    .map(|span| [span.start, span.end]),
                snippet: command_snippet(&entry.text),
            })
        })
        .collect::<Vec<_>>();

    let mut line_groups = vec![None; snapshot.decompile.text.lines().count()];
    let mut line_source_map = vec![None; snapshot.decompile.text.lines().count()];
    for (source_map_index, entry) in snapshot.source_map.entries().iter().enumerate() {
        for line in entry.source_lines.clone() {
            if let Some(slot) = line_groups.get_mut(line) {
                *slot = entry.command_group();
            }
            if let Some(slot) = line_source_map.get_mut(line) {
                *slot = Some(source_map_index);
            }
        }
    }

    LoadResult {
        metadata: FileMetadataDTO {
            filename: snapshot.metadata.filename.clone(),
            path: snapshot.metadata.path.clone(),
            tcl_version: snapshot.metadata.tcl_version.clone(),
            instruction_count: snapshot.metadata.instruction_count,
            literal_count: snapshot.metadata.literal_count,
            command_count: snapshot.metadata.command_count,
            exception_count: snapshot.metadata.exception_count,
            aux_count: snapshot.metadata.aux_count,
            code_bytes: snapshot.metadata.code_bytes,
            max_stack_depth: snapshot.metadata.max_stack_depth,
            syntax_engine: syntax_engine.to_string(),
            format: snapshot.metadata.format,
            build: snapshot.metadata.build,
        },
        instructions: snapshot
            .instructions
            .iter()
            .map(|inst| InstructionDTO {
                idx: inst.idx,
                pc: inst.pc,
                op: inst.op.clone(),
                args: inst
                    .operands
                    .iter()
                    .map(|operand| operand.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                operands: inst.operands.clone(),
                argc: inst.argc,
                group: inst.group,
                command_pc_range: inst.command_pc_range,
                group_start: inst.group_start,
                comment: inst.comment.clone(),
                size: inst.size,
                is_jump: inst.is_jump,
                is_jump_conditional: inst.is_jump_conditional,
                is_jump_unconditional: inst.is_jump_unconditional,
                jump_target: inst.jump_target,
                fallthrough: inst.fallthrough,
                primary_next_location: inst.primary_next_location,
                secondary_next_location: inst.secondary_next_location,
                next_locations: inst.next_locations.clone(),
            })
            .collect(),
        literals: snapshot
            .bytecode
            .literals
            .iter()
            .enumerate()
            .map(|(idx, literal)| LiteralDTO {
                idx,
                kind: format!("{:?}", literal.kind),
                value: literal.value.source_repr(),
            })
            .collect(),
        exceptions: snapshot
            .exceptions
            .iter()
            .map(|ex| ExceptionDTO {
                idx: ex.idx,
                kind: ex.kind.clone(),
                nesting_level: ex.nesting_level,
                code_range: ex.code_range,
                break_offset: ex.break_offset,
                continue_offset: ex.continue_offset,
                catch_offset: ex.catch_offset,
            })
            .collect(),
        blocks: snapshot
            .basic_blocks
            .iter()
            .map(|block| {
                let start = snapshot.instructions.get(block.start);
                let end = snapshot.instructions.get(block.end);
                BasicBlockDTO {
                    id: block.id,
                    start: block.start,
                    end: block.end,
                    start_pc: start.map(|inst| inst.pc).unwrap_or_default(),
                    end_pc: end.map(|inst| inst.pc + inst.size).unwrap_or_default(),
                    group: start
                        .and_then(|inst| inst.group)
                        .map(|group| format!("c{group:03}"))
                        .unwrap_or_else(|| "c---".to_string()),
                    edges: block.edges.clone(),
                    instructions: (block.start..=block.end)
                        .filter_map(|idx| snapshot.instructions.get(idx))
                        .map(|inst| BlockInstructionDTO {
                            pc: inst.pc,
                            name: inst.op.clone(),
                            operands: inst.operands.clone(),
                        })
                        .collect(),
                }
            })
            .collect(),
        traces: snapshot
            .traces
            .iter()
            .map(|trace| TraceFrameDTO {
                instruction_index: trace.instruction_index,
                pc: trace.pc,
                opcode: trace.opcode.clone(),
                argc: trace.argc,
                stack_delta: trace.stack_delta,
                before_depth: trace.before_depth,
                after_depth: trace.after_depth,
                popped: trace.popped.clone(),
                pushed: trace.pushed.clone(),
            })
            .collect(),
        decompile_lines: snapshot
            .decompile
            .text
            .lines()
            .enumerate()
            .map(|(line_index, text)| DecompileLineDTO {
                text: text.to_string(),
                group: line_groups.get(line_index).copied().flatten(),
                line_index,
                source_map_index: line_source_map.get(line_index).copied().flatten(),
            })
            .collect(),
        decompile_text: snapshot.decompile.text.clone(),
        decompile_tokens: tcld_syntax::highlight_tcl(&snapshot.decompile.text)
            .unwrap_or_default()
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
            .collect(),
        decompile_folding_ranges: tcld_syntax::folding_ranges(&snapshot.decompile.text)
            .unwrap_or_default()
            .into_iter()
            .map(folding_range_to_dto)
            .collect(),
        dump_text: snapshot.dump_text.clone(),
        summary_text: snapshot.summary_text.clone(),
        source_map,
        source_command_anchors,
        diagnostics: DiagnosticsReportDTO {
            reconstruction: format!("{:?}", snapshot.diagnostics.reconstruction),
            recognized_constructs: snapshot.diagnostics.recognized_constructs.clone(),
            unreduced_opcodes: snapshot.diagnostics.unreduced_opcodes.clone(),
            approximations: snapshot
                .diagnostics
                .approximations
                .iter()
                .map(|approximation| DiagnosticApproximationDTO {
                    kind: format!("{:?}", approximation.kind),
                    bytecode_range: approximation
                        .bytecode_range
                        .as_ref()
                        .map(|range| [range.start, range.end]),
                    reason: approximation.reason.clone(),
                })
                .collect(),
            suppressed_fast_path_ranges: snapshot
                .diagnostics
                .suppressed_fast_path_ranges
                .iter()
                .map(|range| [range.start, range.end])
                .collect(),
            pass_decisions: snapshot
                .diagnostics
                .pass_decisions
                .iter()
                .map(|decision| PassDecisionDTO {
                    name: decision.name.clone(),
                    enabled: decision.enabled,
                    iteration: decision.iteration,
                    changed: decision.changed,
                })
                .collect(),
            unsupported_instructions: snapshot.diagnostics.unsupported_instructions.clone(),
            suggested_next_work: snapshot.diagnostics.suggested_next_work.clone(),
        },
        reconstruction: format!("{:?}", snapshot.diagnostics.reconstruction),
        provenance: LoadProvenanceDTO {
            kind: format!("{:?}", snapshot.provenance.kind),
            label: snapshot.provenance.label.clone(),
            original_source_available: snapshot.provenance.original_source_available,
        },
        original_source_text: snapshot.original_source.clone(),
    }
}
