use super::protocol::{AgentCommand, DebugWatchpointHitEvent};
use super::{
    BackendKind, Breakpoint, BytecodeHookMode, DebugLaunchOptions, DebugState, StoppedEventParts,
    build_clear_watchpoint_command, build_debug_stopped_event, build_embedded_launch_plan,
    build_eval_command, build_external_launch_plan, build_get_stack_command,
    build_get_variables_command, build_paused_snapshot_commands, build_resolve_command,
    build_set_watchpoint_command, command_for_external_launch, resolve_hook_plan,
    set_breakpoint_enabled_state,
};

#[test]
fn breakpoint_enabled_preserves_condition_without_recreate() {
    let file = "/tmp/app.tcl".to_string();
    let mut breakpoints = vec![Breakpoint {
        file: file.clone(),
        line: 42,
        offset: None,
        len: None,
        condition: Some("$x > 1".into()),
        enabled: true,
    }];

    set_breakpoint_enabled_state(&mut breakpoints, file.clone(), 42, None, false).unwrap();
    assert_eq!(breakpoints[0].condition.as_deref(), Some("$x > 1"));
    assert!(!breakpoints[0].enabled);

    set_breakpoint_enabled_state(&mut breakpoints, file.clone(), 42, None, true).unwrap();
    assert_eq!(breakpoints[0].condition.as_deref(), Some("$x > 1"));
    assert!(breakpoints[0].enabled);

    let err = set_breakpoint_enabled_state(&mut breakpoints, file.clone(), 99, None, false)
        .expect_err("missing breakpoint is a precise backend error");
    assert!(err.contains("breakpoint not found"));
    assert!(err.contains(":99"));
}

#[test]
fn watchpoint_hit_metadata_is_embedded_in_stopped_event() {
    let hit = DebugWatchpointHitEvent {
        watch_id: 7,
        name: "counter".into(),
        element: Some("total".into()),
        operation: "write".into(),
        old_value: "1".into(),
        new_value: "2".into(),
        file: "/tmp/app.tcl".into(),
        line: 9,
        frame_count: 3,
    };
    let stopped = build_debug_stopped_event(
        StoppedEventParts {
            reason: "watchpoint".into(),
            file: hit.file.clone(),
            line: hit.line,
            frame_count: hit.frame_count,
            pc: Some(12),
            instruction_index: Some(4),
            block_id: Some(2),
            error: None,
            error_info: None,
        },
        Some(&hit),
    );

    assert_eq!(stopped.watchpoint_name.as_deref(), Some("counter"));
    assert_eq!(stopped.watchpoint_element.as_deref(), Some("total"));
    assert_eq!(stopped.watchpoint_old_value.as_deref(), Some("1"));
    assert_eq!(stopped.watchpoint_new_value.as_deref(), Some("2"));
    assert_eq!(stopped.frame_count, 3);
}

#[test]
fn stack_variables_eval_follow_active_paused_frame() {
    let variables = build_get_variables_command(DebugState::Paused, 11, 2).unwrap();
    let stack = build_get_stack_command(DebugState::Paused, 12).unwrap();
    let eval = build_eval_command(DebugState::Paused, 13, "expr {$counter + 1}".into(), 2).unwrap();

    assert!(matches!(
        variables,
        AgentCommand::GetVariables {
            request_id: 11,
            level: 2
        }
    ));
    assert!(matches!(stack, AgentCommand::GetStack { request_id: 12 }));
    assert!(matches!(
        eval,
        AgentCommand::Eval {
            request_id: 13,
            ref script,
            level: 2
        } if script == "expr {$counter + 1}"
    ));

    assert_eq!(
        build_get_variables_command(DebugState::Running, 1, 2).unwrap_err(),
        "not paused (state: Running)"
    );
    assert_eq!(
        build_get_stack_command(DebugState::Idle, 1).unwrap_err(),
        "not paused (state: Idle)"
    );
    assert_eq!(
        build_eval_command(DebugState::Connected, 1, "expr 1".into(), 0).unwrap_err(),
        "not paused (state: Connected)"
    );
}

#[test]
fn stopped_pause_builds_stack_and_variable_snapshot_commands() {
    let commands = build_paused_snapshot_commands(DebugState::Paused, 30, 31, 1).unwrap();
    assert_eq!(commands.len(), 2);
    assert!(matches!(
        commands[0],
        AgentCommand::GetStack { request_id: 30 }
    ));
    assert!(matches!(
        commands[1],
        AgentCommand::GetVariables {
            request_id: 31,
            level: 1
        }
    ));

    assert_eq!(
        build_paused_snapshot_commands(DebugState::Running, 30, 31, 1).unwrap_err(),
        "not paused (state: Running)"
    );
}

#[test]
fn launch_plans_honor_env_cwd_tclsh_and_hook_settings() {
    let options = DebugLaunchOptions {
        bytecode_hook_mode: BytecodeHookMode::Intercept,
        bytecode_hook_commands: vec!["tbcload::bceval".into()],
        env: [("TCLLIBPATH".into(), "/tmp/lib".into())].into(),
        cwd: Some("/tmp/work".into()),
        ..Default::default()
    };
    let hook_plan = resolve_hook_plan(BackendKind::Embedded, options.clone()).unwrap();
    let embedded = build_embedded_launch_plan(
        &[Breakpoint {
            file: "/tmp/app.tcl".into(),
            line: 42,
            offset: None,
            len: None,
            condition: Some("$x > 1".into()),
            enabled: true,
        }],
        "/tmp/app.tcl".into(),
        vec!["arg1".into()],
        hook_plan,
        Some("/tmp/libtcld_dbg.dylib".into()),
        options.clone(),
    );

    assert_eq!(embedded.path, "/tmp/app.tcl");
    assert_eq!(embedded.args, vec!["arg1"]);
    assert_eq!(
        embedded.env.get("TCLLIBPATH").map(String::as_str),
        Some("/tmp/lib")
    );
    assert_eq!(embedded.cwd.as_deref(), Some("/tmp/work"));
    assert_eq!(embedded.hook_plan.mode, BytecodeHookMode::Intercept);
    assert_eq!(embedded.hook_plan.commands, vec!["::tbcload::bceval"]);
    assert_eq!(embedded.breakpoints[0].2.as_deref(), Some("$x > 1"));

    let external_hook_plan = resolve_hook_plan(BackendKind::External, options.clone()).unwrap();
    let external = build_external_launch_plan(
        "/tmp/app.tcl".into(),
        vec!["arg1".into()],
        Some("/custom/bin/tclsh".into()),
        external_hook_plan,
        Some("/tmp/libtcld_dbg.dylib".into()),
        options,
        "/tmp/tcld-agent.tcl".into(),
        4567,
    );
    let command = command_for_external_launch(&external);
    let std_command = command.as_std();
    let args = std_command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let envs = std_command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        std_command.get_program().to_string_lossy(),
        "/custom/bin/tclsh"
    );
    assert_eq!(
        std_command
            .get_current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .as_deref(),
        Some("/tmp/work")
    );
    assert!(envs.contains(&("TCLLIBPATH".into(), Some("/tmp/lib".into()))));
    assert!(
        args.windows(2)
            .any(|window| window[0] == "--port" && window[1] == "4567")
    );
    assert!(
        args.windows(2)
            .any(|window| window[0] == "--bytecode-hook-mode" && window[1] == "intercept")
    );
    assert!(
        args.windows(2).any(
            |window| window[0] == "--bytecode-hook-command" && window[1] == "::tbcload::bceval"
        )
    );
    assert!(
        args.windows(2)
            .any(|window| window[0] == "--script" && window[1] == "/tmp/app.tcl")
    );
    assert!(args.contains(&"arg1".into()));
}

#[test]
fn command_resolution_and_watchpoints_reject_invalid_backend_state_precisely() {
    assert_eq!(
        build_resolve_command(DebugState::Idle, 1, "puts".into(), Some("::".into())).unwrap_err(),
        "not paused (state: Idle)"
    );
    assert_eq!(
        build_set_watchpoint_command(
            DebugState::Running,
            1,
            7,
            "counter".into(),
            Some("local".into()),
            Some(2),
            None,
        )
        .unwrap_err(),
        "not paused (state: Running)"
    );
    assert_eq!(
        build_clear_watchpoint_command(DebugState::Connected, 1, "counter".into()).unwrap_err(),
        "not paused (state: Connected)"
    );
    assert_eq!(
        build_set_watchpoint_command(
            DebugState::Paused,
            1,
            7,
            "counter".into(),
            Some("bad-scope".into()),
            Some(2),
            None,
        )
        .unwrap_err(),
        "invalid watchpoint scope 'bad-scope' (expected local, global, or namespace)"
    );

    let resolve =
        build_resolve_command(DebugState::Paused, 21, "puts".into(), Some("::".into())).unwrap();
    assert!(matches!(
        resolve,
        AgentCommand::Eval {
            request_id: 21,
            ref script,
            level: 0
        } if script == "::tcld::dbg::resolve_command {puts} {::}"
    ));

    let (watch_command, watchpoint) = build_set_watchpoint_command(
        DebugState::Paused,
        22,
        8,
        "counter".into(),
        Some("namespace".into()),
        Some(2),
        Some("$counter > 1".into()),
    )
    .unwrap();
    assert!(matches!(
        watch_command,
        AgentCommand::Eval {
            request_id: 22,
            ref script,
            level: 0
        } if script.contains("::tcld::dbg::watch_variable {counter} -scope {namespace}")
            && script.contains("-level 2")
            && script.contains("-condition {$counter > 1}")
    ));
    assert_eq!(watchpoint.id, 8);
    assert_eq!(watchpoint.scope, "namespace");
    assert_eq!(watchpoint.level, Some(2));
}
