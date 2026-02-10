use tcld_interp::Interpreter;

#[test]
fn eval_simple_expression() {
    let interp = Interpreter::new().expect("create interp");
    let result = interp.eval("expr {2 + 3}").unwrap();
    assert_eq!(result, "5");
}

#[test]
fn eval_set_and_get_var() {
    let interp = Interpreter::new().expect("create interp");
    interp.set_var("x", "hello").unwrap();
    let val = interp.get_var("x");
    assert_eq!(val.as_deref(), Some("hello"));
}

#[test]
fn eval_file_not_found() {
    let interp = Interpreter::new().expect("create interp");
    let result = interp.eval_file("/nonexistent/file.tcl");
    assert!(result.is_err());
}

#[test]
fn trace_fires_on_commands() {
    use std::sync::{Arc, Mutex};

    let commands = Arc::new(Mutex::new(Vec::new()));
    let commands_clone = commands.clone();

    let mut interp = Interpreter::new().expect("create interp");
    interp.set_trace(move |cmd, _level| {
        commands_clone.lock().unwrap().push(cmd.to_string());
        true
    });

    interp.eval("set x 1").unwrap();
    interp.eval("set y 2").unwrap();

    let cmds = commands.lock().unwrap();
    assert!(
        cmds.iter().any(|c| c.contains("set x 1")),
        "trace should capture 'set x 1', got: {:?}",
        *cmds
    );
    assert!(
        cmds.iter().any(|c| c.contains("set y 2")),
        "trace should capture 'set y 2', got: {:?}",
        *cmds
    );
}

#[test]
fn trace_abort_stops_command() {
    let mut interp = Interpreter::new().expect("create interp");

    // Abort any command containing "forbidden"
    interp.set_trace(move |cmd, _level| !cmd.contains("forbidden"));

    // Normal command should work
    let result = interp.eval("set x 1");
    assert!(result.is_ok());

    // Command containing "forbidden" should fail
    let result = interp.eval("set forbidden 1");
    assert!(result.is_err());
}

#[test]
fn drop_cleans_up() {
    // Just ensure no crash on drop
    let mut interp = Interpreter::new().expect("create interp");
    interp.set_trace(|_, _| true);
    drop(interp);
}
