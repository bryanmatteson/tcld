use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const ROWS: u16 = 32;
const COLS: u16 = 120;

#[test]
fn explore_tui_handles_core_interactions_under_real_pty() {
    let input = env::var_os("TCLD_EXPLORE_PTY_INPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../testing/fixtures/public/public_complex_fixture.tbc")
        });
    let input = absolutize(&input).expect("resolve pty input path");
    let frame_path = env::var_os("TCLD_EXPLORE_PTY_FRAME").map(PathBuf::from);
    let mut pty = ExplorePty::spawn(&input, frame_path.as_deref());

    pty.capture("browse_initial", Duration::from_secs(10), |text| {
        assert_browse(text)?;
        if text.contains("screen 1/1") {
            return Err("TurtleTap Shell chrome leaked into the product viewport".to_string());
        }
        require(text, "Canvas - Instructions")?;
        require(text, "Instructions  #1/")
    });

    pty.send(b"\x1b[6~");
    pty.capture("product_page_down", Duration::from_secs(2), |text| {
        assert_browse(text)?;
        if text.contains("Instructions  #1/") {
            return Err("product-owned PageDown did not move the selection".to_string());
        }
        require(text, "Instructions  #")
    });
    pty.send(b"\x1b[5~");
    pty.capture("product_page_up", Duration::from_secs(2), |text| {
        assert_browse(text)?;
        require(text, "Instructions  #1/")
    });

    pty.send(&[b'j'; 50]);
    pty.capture("browse_follow_scroll", Duration::from_secs(3), |text| {
        assert_browse(text)?;
        require(text, "Instructions  #51/")?;
        require(text, "▎")
    });

    pty.send(b"\x1b[<0;40;18M");
    pty.capture("browse_canvas_mouse", Duration::from_secs(2), |text| {
        assert_browse(text)?;
        if text.contains("Instructions  #51/") {
            return Err("clicking a visible canvas row did not update selection".to_string());
        }
        require(text, "workspace rail focused").or_else(|_| require(text, "Instructions  #"))
    });

    pty.send(b"i");
    pty.capture("instructions_view", Duration::from_secs(2), |text| {
        require(text, "Canvas - Instructions")?;
        assert_token_in_canvas(text, "push1")
    });

    pty.send(b"l");
    pty.capture("literals_view", Duration::from_secs(2), |text| {
        require(text, "Canvas - Literals")?;
        assert_token_in_canvas(text, "XString package")
    });

    pty.send(b"e");
    pty.capture("exceptions_view", Duration::from_secs(2), |text| {
        require(text, "Canvas - Exceptions")?;
        assert_token_in_canvas(text, "No entries in Exceptions")
    });

    pty.send(b"d");
    pty.capture("decompile_view", Duration::from_secs(2), |text| {
        assert_browse(text)?;
        require(text, "Canvas - Decompile")?;
        assert_token_in_canvas(text, "package provide tcld_fixture")
    });

    pty.send(b"c");
    pty.capture("source_view", Duration::from_secs(2), |text| {
        assert_browse(text)?;
        require(text, "Canvas - Source")?;
        assert_token_in_canvas(text, "package provide tcld_fixture")
    });

    pty.send(b":diff tests/fixtures/expr.tbc\r");
    pty.capture("diff_opened", Duration::from_secs(2), |text| {
        require(text, "diff opened: tests/fixtures/expr.tbc")
    });

    pty.send(b"C");
    pty.capture("compare_mode", Duration::from_secs(2), |text| {
        assert_static_only(text)?;
        require(text, "[C] Compare")?;
        require(text, "Left Source / Decompile")?;
        require(text, "Diff Report")?;
        require(text, "Next")
    });

    pty.send(b"]");
    pty.capture("compare_next_change", Duration::from_secs(2), |text| {
        require(text, "diff change 1/")
    });

    pty.send(b"b");
    pty.capture("browse_return", Duration::from_secs(2), assert_browse);

    pty.send(b"?");
    pty.capture("help_overlay", Duration::from_secs(2), |text| {
        assert_static_only(text)?;
        require(text, "Quick Reference")?;
        require(text, "Modes")?;
        require(text, "Movement")?;
        require(text, "Views")?;
        require(text, "C       Compare")
    });

    pty.send(b"\x1b");
    pty.send(b"\x10");
    pty.send(b"decompile");
    pty.capture("palette_overlay", Duration::from_secs(2), |text| {
        assert_static_only(text)?;
        require(text, "Command Palette")?;
        require(text, "> Open Decompile view")
    });

    pty.send(b"\x1b");
    pty.send(b":rdebug\r");
    pty.capture("rdebug_migration", Duration::from_secs(2), |text| {
        require(text, "runtime debugging moved to tcld debug <file>")
    });

    pty.resize(19, 70);
    pty.capture("minimum_size_guard", Duration::from_secs(2), |text| {
        require(text, "Terminal too small: 70x19")?;
        require(text, "Required: 72x20")?;
        require(text, "Ctrl-C exit | q quit")
    });
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(3));
}

#[test]
fn explore_large_repetitive_source_search_is_bounded_under_real_pty() {
    let fixture_dir = tempfile::tempdir().expect("create large source fixture directory");
    let input = fixture_dir.path().join("repetitive.tcl");
    let source = (0..1_500)
        .map(|index| format!("set repeated {index}\n"))
        .collect::<String>();
    fs::write(&input, source).expect("write large repetitive source fixture");

    let mut pty = ExplorePty::spawn(&input, None);
    pty.capture("large_source_initial", Duration::from_secs(10), |text| {
        require(text, "tcld explore")
    });
    pty.send(b"c");
    pty.capture("large_source_view", Duration::from_secs(2), |text| {
        require(text, "Canvas - Source")
    });
    pty.send(b":search repeated\r");
    pty.capture(
        "large_source_bounded_search",
        Duration::from_secs(3),
        |text| {
            require(
                text,
                "1500 match(es) for 'repeated'; navigating the first 1000",
            )
        },
    );
    pty.send(b"n");
    pty.capture(
        "large_source_search_navigation",
        Duration::from_secs(2),
        |text| require(text, "match 2/1000"),
    );
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(3));
}

#[test]
fn explore_panic_is_printed_after_terminal_restore() {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testing/fixtures/public/public_complex_fixture.tbc");
    let input = absolutize(&input).expect("resolve panic fixture");
    let mut pty = ExplorePty::spawn_shell();
    let command = format!(
        "{} explore {}\r",
        shell_quote(Path::new(env!("CARGO_BIN_EXE_tcld"))),
        shell_quote(&input)
    );
    pty.send(command.as_bytes());
    pty.capture("panic_after_restore", Duration::from_secs(10), |text| {
        require(text, "panic at ")?;
        require(text, "intentional explore terminal panic")
    });

    let probe = "printf 'TCLD_PTY_RESTORED\\n'";
    pty.send(probe.as_bytes());
    pty.capture("shell_echo_restored", Duration::from_secs(2), |text| {
        require(text, probe)
    });
    pty.send(b"\r");
    pty.capture(
        "shell_command_after_panic",
        Duration::from_secs(2),
        |text| require(text, "TCLD_PTY_RESTORED"),
    );
    pty.send(b"exit\r");
    pty.wait_for_exit(Duration::from_secs(2));
}

struct ExplorePty {
    parser: vt100::Parser,
    cols: u16,
    _home: tempfile::TempDir,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    captures: Vec<(&'static str, String)>,
    frame_path: Option<PathBuf>,
}

impl ExplorePty {
    fn spawn(input: &Path, frame_path: Option<&Path>) -> Self {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tcld"));
        cmd.arg("explore");
        cmd.arg(input);
        cmd.env("TERM", "xterm-256color");
        cmd.cwd(env!("CARGO_MANIFEST_DIR"));
        Self::spawn_command(cmd, frame_path)
    }

    fn spawn_shell() -> Self {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.env("TERM", "xterm-256color");
        cmd.env("PS1", "tcld-test$ ");
        cmd.env("RUST_BACKTRACE", "0");
        cmd.env("TCLD_TEST_PANIC_AFTER_TERMINAL_ENTER", "explore");
        cmd.cwd(env!("CARGO_MANIFEST_DIR"));
        Self::spawn_command(cmd, None)
    }

    fn spawn_command(mut cmd: CommandBuilder, frame_path: Option<&Path>) -> Self {
        let home = tempfile::tempdir().expect("create isolated PTY home");
        cmd.env("HOME", home.path());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let child = pair.slave.spawn_command(cmd).expect("spawn tcld explore");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
        let writer = pair.master.take_writer().expect("take pty writer");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            parser: vt100::Parser::new(ROWS, COLS, 0),
            cols: COLS,
            _home: home,
            master: pair.master,
            child,
            writer,
            rx,
            captures: Vec::new(),
            frame_path: frame_path.map(Path::to_path_buf),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write to pty");
        self.writer.flush().expect("flush pty");
        self.pump(Duration::from_millis(120));
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize PTY");
        self.parser.set_size(rows, cols);
        self.cols = cols;
        self.pump(Duration::from_millis(150));
    }

    fn capture<F>(&mut self, label: &'static str, timeout: Duration, check: F) -> String
    where
        F: Fn(&str) -> CheckResult,
    {
        let deadline = Instant::now() + timeout;

        loop {
            self.pump(Duration::from_millis(50));
            let text = self.screen_text();
            match check(&text) {
                Ok(()) => {
                    self.captures.push((label, text.clone()));
                    self.write_captures();
                    return text;
                }
                Err(err) if Instant::now() >= deadline => {
                    panic!("timed out waiting for {label}: {err}\n{text}");
                }
                Err(_) => {}
            }
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(50));
            if self.child.try_wait().expect("poll child").is_some() {
                return;
            }
        }
        panic!("tcld explore did not exit after quit confirmation");
    }

    fn pump(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Duration::from_millis(20));
            match self.rx.recv_timeout(wait) {
                Ok(bytes) => self.parser.process(&bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn screen_text(&self) -> String {
        let mut rows = self
            .parser
            .screen()
            .rows(0, self.cols)
            .map(|row| row.trim_end().to_string())
            .collect::<Vec<_>>();
        while rows.last().is_some_and(|row| row.is_empty()) {
            rows.pop();
        }
        let mut text = rows.join("\n");
        text.push('\n');
        text
    }

    fn write_captures(&self) {
        if let Some(path) = &self.frame_path {
            let mut out = String::new();
            for (label, text) in &self.captures {
                out.push_str("===== ");
                out.push_str(label);
                out.push_str(" =====\n");
                out.push_str(text);
                out.push('\n');
            }
            fs::write(path, out).expect("write pty frame capture");
        }
    }
}

impl Drop for ExplorePty {
    fn drop(&mut self) {
        self.write_captures();
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

type CheckResult = Result<(), String>;

fn absolutize(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        path.canonicalize()
    } else {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest_dir
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let candidates = [Some(env::current_dir()?), Some(manifest_dir), workspace_dir];

        for base in candidates.into_iter().flatten() {
            let candidate = base.join(path);
            if let Ok(path) = candidate.canonicalize() {
                return Ok(path);
            }
        }

        path.canonicalize()
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

fn assert_browse(text: &str) -> CheckResult {
    assert_static_only(text)?;
    require(text, "tcld explore")?;
    require(text, "[b] Browse")?;
    assert_after(text, "Workspace", "[b] Browse")?;
    assert_same_row_order(text, "Workspace", "Canvas", "Inspector")?;
    require(text, ": Command").or_else(|_| require(text, ": Cmd"))?;
    assert_no_old_grid(text)
}

fn assert_static_only(text: &str) -> CheckResult {
    for removed in ["Runtime Debug", "rdebug", "Mode: Debug", "Graph", " Dump "] {
        if text.contains(removed) {
            return Err(format!(
                "removed Explore surface {removed:?} is visible\n{text}"
            ));
        }
    }
    Ok(())
}

fn assert_no_old_grid(text: &str) -> CheckResult {
    if text.contains("Tabs:")
        || text
            .lines()
            .any(|line| line.trim_start().starts_with("Tabs:"))
    {
        return Err(format!("old tab grid is still visible\n{text}"));
    }
    Ok(())
}

fn require(text: &str, needle: &str) -> CheckResult {
    locate(text, needle).map(|_| ())
}

fn assert_after(text: &str, later: &str, earlier: &str) -> CheckResult {
    let (later_row, _) = locate(text, later)?;
    let (earlier_row, _) = locate(text, earlier)?;
    if later_row > earlier_row {
        Ok(())
    } else {
        Err(format!("expected {later:?} after {earlier:?}\n{text}"))
    }
}

fn assert_same_row_order(text: &str, left: &str, middle: &str, right: &str) -> CheckResult {
    let (left_row, left_col) = locate(text, left)?;
    let (middle_row, middle_col) = locate(text, middle)?;
    let (right_row, right_col) = locate(text, right)?;
    if left_row == middle_row
        && middle_row == right_row
        && left_col < middle_col
        && middle_col < right_col
    {
        Ok(())
    } else {
        Err(format!(
            "expected {left:?} < {middle:?} < {right:?} on one row\n{text}"
        ))
    }
}

fn assert_token_in_canvas(text: &str, token: &str) -> CheckResult {
    let canvas_line = text
        .lines()
        .find(|line| line.contains("Canvas -"))
        .ok_or_else(|| format!("missing canvas title\n{text}"))?;
    let canvas_byte = canvas_line.find("Canvas -").expect("canvas title found");
    let canvas_col = canvas_line[..canvas_byte].chars().count().saturating_sub(2);
    let inspector_line = text
        .lines()
        .find(|line| line.contains("Inspector"))
        .ok_or_else(|| format!("missing inspector title\n{text}"))?;
    let inspector_byte = inspector_line
        .find("Inspector")
        .expect("inspector title found");
    let inspector_col = inspector_line[..inspector_byte]
        .chars()
        .count()
        .saturating_sub(2);
    for line in text.lines() {
        let mut start = 0;
        while let Some(relative) = line[start..].find(token) {
            let byte = start + relative;
            let col = line[..byte].chars().count();
            if canvas_col <= col && col < inspector_col {
                return Ok(());
            }
            start = byte + token.len();
        }
    }
    Err(format!(
        "missing {token:?} inside canvas columns {canvas_col}..{inspector_col}\n{text}"
    ))
}

fn locate(text: &str, needle: &str) -> Result<(usize, usize), String> {
    for (row, line) in text.lines().enumerate() {
        if let Some(col) = line.find(needle) {
            return Ok((row, col));
        }
    }
    Err(format!("missing {needle:?}\n{text}"))
}
