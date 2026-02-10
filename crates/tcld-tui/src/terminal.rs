use std::any::Any;
use std::backtrace::Backtrace;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use turtletap::{
    Event, Frame, Rect, RuntimeAction, RuntimeEvent, TerminalApplication, TerminalConfig,
};
use turtletap::{KeyEventKind, TerminalRuntime};

pub type TerminalError = Box<dyn Error>;

#[derive(Debug)]
struct PanicReport {
    payload: String,
    location: String,
    backtrace: Option<String>,
}

/// Attach a Tcld application through TurtleTap's same-thread terminal runtime.
///
/// TurtleTap owns raw mode, alternate-screen state, cursor restoration, mouse
/// capture, ticks, and resize delivery. The Tcld decorator preserves the
/// application's complete TurtleTap contract while filtering key-release
/// noise, redrawing on focus gain, and reporting panics after terminal restore.
pub fn run<A>(application: &str, app: A) -> Result<A::Exit, TerminalError>
where
    A: TerminalApplication,
{
    run_with_config(
        application,
        app,
        TerminalConfig::new()
            .with_tick_rate(Duration::from_millis(50))
            .with_mouse_capture(true),
    )
}

/// Attach a Tcld application with explicit TurtleTap terminal configuration.
pub fn run_with_config<A>(
    application: &str,
    app: A,
    config: TerminalConfig,
) -> Result<A::Exit, TerminalError>
where
    A: TerminalApplication,
{
    let report = Arc::new(Mutex::new(None));
    let report_for_hook = Arc::clone(&report);
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = panic_payload(info.payload());
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_string());
        let backtrace = backtrace_requested().then(|| Backtrace::force_capture().to_string());
        if let Ok(mut slot) = report_for_hook.lock() {
            *slot = Some(PanicReport {
                payload,
                location,
                backtrace,
            });
        }
    }));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut app = TcldApplication::new(application, app);
        TerminalRuntime::new(config)
            .run(&mut app)
            .map_err(|error| Box::new(error) as TerminalError)
    }));
    std::panic::set_hook(previous_hook);

    match outcome {
        Ok(result) => result,
        Err(payload) => {
            let captured = report.lock().ok().and_then(|mut slot| slot.take());
            if let Some(captured) = captured {
                eprintln!("panic at {}: {}", captured.location, captured.payload);
                if let Some(backtrace) = captured.backtrace {
                    eprintln!("stack backtrace:\n{backtrace}");
                }
            } else {
                eprintln!(
                    "panic after terminal entry: {}",
                    panic_payload(payload.as_ref())
                );
            }
            std::panic::resume_unwind(payload);
        }
    }
}

struct TcldApplication<'a, A> {
    application: &'a str,
    panic_checked: bool,
    inner: A,
}

impl<'a, A> TcldApplication<'a, A> {
    fn new(application: &'a str, inner: A) -> Self {
        Self {
            application,
            panic_checked: false,
            inner,
        }
    }
}

impl<A: TerminalApplication> TerminalApplication for TcldApplication<'_, A> {
    type Exit = A::Exit;

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if !self.panic_checked {
            self.panic_checked = true;
            if std::env::var("TCLD_TEST_PANIC_AFTER_TERMINAL_ENTER").as_deref()
                == Ok(self.application)
            {
                panic!("intentional {} terminal panic", self.application);
            }
        }
        self.inner.render(frame, area);
    }

    fn handle(&mut self, event: RuntimeEvent) -> RuntimeAction<Self::Exit> {
        match event {
            RuntimeEvent::Terminal(Event::Key(key))
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
            {
                RuntimeAction::Ignored
            }
            RuntimeEvent::Terminal(Event::FocusGained) => RuntimeAction::Redraw,
            event => self.inner.handle(event),
        }
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.inner.poll_interval()
    }
}

fn panic_payload(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn backtrace_requested() -> bool {
    std::env::var("RUST_BACKTRACE")
        .map(|value| value != "0" && !value.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use crossterm::event::{KeyCode, KeyEvent, KeyEventState, KeyModifiers};

    use super::*;

    #[test]
    fn panic_payload_formats_common_payloads() {
        assert_eq!(panic_payload(&"boom"), "boom");
        assert_eq!(panic_payload(&"boom".to_string()), "boom");
    }

    struct LocalProbe {
        ticks: Rc<Cell<u32>>,
    }

    impl TerminalApplication for LocalProbe {
        type Exit = u32;

        fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect) {}

        fn handle(&mut self, event: RuntimeEvent) -> RuntimeAction<Self::Exit> {
            if matches!(event, RuntimeEvent::Tick(_)) {
                self.ticks.set(self.ticks.get() + 1);
                RuntimeAction::ClearAndRedraw
            } else {
                RuntimeAction::Exit(7)
            }
        }

        fn poll_interval(&self) -> Option<Duration> {
            Some(Duration::from_millis(25))
        }
    }

    #[test]
    fn runtime_decorator_preserves_the_complete_non_send_application_contract() {
        let ticks = Rc::new(Cell::new(0));
        let mut app = TcldApplication::new(
            "test",
            LocalProbe {
                ticks: Rc::clone(&ticks),
            },
        );

        assert!(matches!(
            TerminalApplication::handle(&mut app, RuntimeEvent::Tick(Duration::from_millis(50))),
            RuntimeAction::ClearAndRedraw
        ));
        assert_eq!(ticks.get(), 1);
        assert_eq!(
            TerminalApplication::poll_interval(&app),
            Some(Duration::from_millis(25))
        );
        assert!(matches!(
            TerminalApplication::handle(&mut app, RuntimeEvent::Terminal(Event::FocusLost)),
            RuntimeAction::Exit(7)
        ));
        assert!(matches!(
            TerminalApplication::handle(&mut app, RuntimeEvent::Terminal(Event::FocusGained)),
            RuntimeAction::Redraw
        ));
        assert!(matches!(
            TerminalApplication::handle(
                &mut app,
                RuntimeEvent::Terminal(Event::Key(KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Release,
                    state: KeyEventState::NONE,
                }))
            ),
            RuntimeAction::Ignored
        ));
    }
}
