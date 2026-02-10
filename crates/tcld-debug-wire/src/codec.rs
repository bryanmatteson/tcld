//! Length-prefixed framing on any `Read + Write`.
//!
//! On-wire layout per frame:
//!
//! ```text
//! <decimal-byte-length>\n<tcl-list-body>\n
//! ```
//!
//! The length counts the bytes of `<tcl-list-body>` only (not the
//! trailing newline). The decimal length itself is ASCII digits up
//! to 16 chars. Length prefix is required because Tcl lists may
//! contain literal newlines in braced elements; pure line-orientation
//! is insufficient.

use std::io::{self, Read, Write};

use crate::frame::{FrameParseError, WireFrame};

/// Maximum digits we'll accept in the length prefix. 10 digits ⇒
/// 9.99 GB body. Anything bigger is treated as protocol violation.
const MAX_LENGTH_DIGITS: usize = 10;

/// Hard cap on body bytes per frame. v2 implementations MUST refuse
/// frames larger than this. 32 MiB is generous for any plausible
/// payload (proc bodies, stack traces, watch values).
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
pub enum CodecError {
    Io(io::Error),
    BadLengthPrefix(String),
    BodyTooLarge { announced: usize, max: usize },
    Frame(FrameParseError),
    UnexpectedEof,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::BadLengthPrefix(s) => write!(f, "bad length prefix: {s:?}"),
            Self::BodyTooLarge { announced, max } => {
                write!(f, "body too large: {announced} bytes (max {max})")
            }
            Self::Frame(e) => write!(f, "frame parse error: {e}"),
            Self::UnexpectedEof => write!(f, "unexpected EOF"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<io::Error> for CodecError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Write a frame to `w`. Performs at most two writes: length prefix
/// (with trailing newline), then body (with trailing newline).
pub fn encode_frame<W: Write>(w: &mut W, frame: &WireFrame) -> Result<(), CodecError> {
    let body = frame.to_wire_body();
    let body_bytes = body.as_bytes();
    if body_bytes.len() > MAX_BODY_BYTES {
        return Err(CodecError::BodyTooLarge {
            announced: body_bytes.len(),
            max: MAX_BODY_BYTES,
        });
    }
    let prefix = format!("{}\n", body_bytes.len());
    w.write_all(prefix.as_bytes())?;
    w.write_all(body_bytes)?;
    w.write_all(b"\n")?;
    Ok(())
}

/// Read one frame from `r`. Blocks until a complete frame is
/// available or EOF. Frame body is limited to `MAX_BODY_BYTES`.
pub fn decode_frame<R: Read>(r: &mut R) -> Result<WireFrame, CodecError> {
    // Read decimal length (ASCII digits) up to '\n' or MAX_LENGTH_DIGITS.
    let mut len_buf = [0u8; MAX_LENGTH_DIGITS + 1];
    let mut filled = 0usize;
    loop {
        if filled == len_buf.len() {
            return Err(CodecError::BadLengthPrefix(format!(
                "no newline within {MAX_LENGTH_DIGITS} digits"
            )));
        }
        let mut one = [0u8; 1];
        match r.read(&mut one) {
            Ok(0) => return Err(CodecError::UnexpectedEof),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(CodecError::Io(e)),
        }
        if one[0] == b'\n' {
            break;
        }
        len_buf[filled] = one[0];
        filled += 1;
    }
    let len_str = std::str::from_utf8(&len_buf[..filled])
        .map_err(|_| CodecError::BadLengthPrefix("non-ascii".into()))?;
    let body_len: usize = len_str
        .parse()
        .map_err(|_| CodecError::BadLengthPrefix(len_str.to_string()))?;
    if body_len > MAX_BODY_BYTES {
        return Err(CodecError::BodyTooLarge {
            announced: body_len,
            max: MAX_BODY_BYTES,
        });
    }
    let mut body = vec![0u8; body_len];
    r.read_exact(&mut body).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            CodecError::UnexpectedEof
        } else {
            CodecError::Io(e)
        }
    })?;
    // Trailing newline.
    let mut nl = [0u8; 1];
    r.read_exact(&mut nl).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            CodecError::UnexpectedEof
        } else {
            CodecError::Io(e)
        }
    })?;
    if nl[0] != b'\n' {
        return Err(CodecError::BadLengthPrefix(format!(
            "expected trailing newline, got byte 0x{:02x}",
            nl[0]
        )));
    }
    let body_str = std::str::from_utf8(&body)
        .map_err(|_| CodecError::Frame(FrameParseError::UnknownKind("non-utf8".into())))?;
    WireFrame::from_wire_body(body_str).map_err(CodecError::Frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameKind;

    #[test]
    fn round_trip_single_frame() {
        let frame = WireFrame::new(FrameKind::Req, 42, "step_into");
        let mut buf = Vec::new();
        encode_frame(&mut buf, &frame).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let parsed = decode_frame(&mut cursor).unwrap();
        assert_eq!(parsed, frame);
    }

    #[test]
    fn round_trip_two_frames_in_sequence() {
        let f1 = WireFrame::new(FrameKind::Req, 1, "step_into");
        let f2 = WireFrame::new(FrameKind::Resp, 1, "ok");
        let mut buf = Vec::new();
        encode_frame(&mut buf, &f1).unwrap();
        encode_frame(&mut buf, &f2).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(decode_frame(&mut cursor).unwrap(), f1);
        assert_eq!(decode_frame(&mut cursor).unwrap(), f2);
    }

    #[test]
    fn rejects_oversized_announce() {
        let too_big = format!("{}\n", MAX_BODY_BYTES + 1);
        let mut cursor = std::io::Cursor::new(too_big.into_bytes());
        let err = decode_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, CodecError::BodyTooLarge { .. }));
    }

    #[test]
    fn rejects_non_numeric_length() {
        let bad = b"abc\n";
        let mut cursor = std::io::Cursor::new(bad.to_vec());
        let err = decode_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, CodecError::BadLengthPrefix(_)));
    }

    #[test]
    fn rejects_eof_before_body() {
        let bad = b"100\nshort";
        let mut cursor = std::io::Cursor::new(bad.to_vec());
        let err = decode_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, CodecError::UnexpectedEof));
    }

    #[test]
    fn rejects_missing_trailing_newline() {
        // Build a body that's followed by 'X' instead of '\n'.
        let body = "req 1 payload";
        let bad = format!("{}\n{}X", body.len(), body);
        let mut cursor = std::io::Cursor::new(bad.into_bytes());
        let err = decode_frame(&mut cursor).unwrap_err();
        assert!(matches!(err, CodecError::BadLengthPrefix(_)));
    }

    #[test]
    fn handles_embedded_newline_in_braced_payload() {
        // Tcl lists allow newlines inside braces; the length prefix
        // is what saves us from line-oriented parsing.
        let frame = WireFrame::new(FrameKind::Req, 1, "eval {set x 1\nset y 2}");
        let mut buf = Vec::new();
        encode_frame(&mut buf, &frame).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let parsed = decode_frame(&mut cursor).unwrap();
        assert_eq!(parsed, frame);
    }
}
