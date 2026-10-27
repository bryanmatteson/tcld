//! The standard output streams vm-strict writes itself: the process's own,
//! or buffers an embedding application reads back, as it reads hybrid mode's
//! host capture.

use std::io::Write;

use crate::error::VmError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    /// The stream a `puts` channel name names.
    pub fn from_channel(name: &str) -> Option<Self> {
        match name {
            "stdout" => Some(Self::Stdout),
            "stderr" => Some(Self::Stderr),
            _ => None,
        }
    }

    fn channel(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Default)]
pub struct Output {
    /// What has been written to stdout and stderr since capture began.
    captured: Option<(Vec<u8>, Vec<u8>)>,
}

impl Output {
    /// Collect what is written from now on instead of passing it on.
    pub fn begin_capture(&mut self) {
        self.captured = Some((Vec::new(), Vec::new()));
    }

    /// The captured stdout and stderr; empty when nothing is being captured.
    pub fn captured(&self) -> (Vec<u8>, Vec<u8>) {
        self.captured.clone().unwrap_or_default()
    }

    /// Write to a stream. The process's stdout is flushed at once, so its
    /// output keeps its order with stderr's.
    pub fn write(&mut self, stream: Stream, bytes: &[u8]) -> Result<(), VmError> {
        if let Some((stdout, stderr)) = self.captured.as_mut() {
            match stream {
                Stream::Stdout => stdout,
                Stream::Stderr => stderr,
            }
            .extend_from_slice(bytes);
            return Ok(());
        }
        let written = match stream {
            Stream::Stdout => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(bytes).and_then(|()| stdout.flush())
            }
            Stream::Stderr => std::io::stderr().lock().write_all(bytes),
        };
        written.map_err(|error| VmError::TclError {
            message: format!(
                "error writing \"{}\": {}",
                stream.channel(),
                posix_message(&error)
            ),
            error_info: None,
            error_code: None,
        })
    }
}

/// `Tcl_PosixError`'s wording of an I/O error: `broken pipe`.
fn posix_message(error: &std::io::Error) -> String {
    let text = error.to_string();
    let text = text.split(" (os error").next().unwrap_or(&text);
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_keeps_the_streams_apart() {
        let mut output = Output::default();
        assert_eq!(output.captured(), (Vec::new(), Vec::new()));
        output.begin_capture();
        output.write(Stream::Stdout, b"out\n").unwrap();
        output.write(Stream::Stderr, b"err").unwrap();
        assert_eq!(output.captured(), (b"out\n".to_vec(), b"err".to_vec()));
        assert_eq!(Stream::from_channel("stdin"), None);
        assert_eq!(
            posix_message(&std::io::Error::from_raw_os_error(32)),
            "broken pipe"
        );
    }
}
