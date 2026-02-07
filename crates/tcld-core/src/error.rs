use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodeScannerError {
    #[error("unexpected end of input at offset {offset}", offset = .offset.unwrap_or(0))]
    Eof { offset: Option<usize> },
    #[error("not found")]
    NotFound,
    #[error("invalid input at offset {offset}", offset = .offset.unwrap_or(0))]
    InvalidInput { offset: Option<usize> },
    #[error("invalid int at offset {offset}", offset = .offset.unwrap_or(0))]
    InvalidInt { offset: Option<usize> },
    #[error("invalid float")]
    InvalidFloat,
    #[error("invalid hex")]
    InvalidHex,
}

impl CodeScannerError {
    /// Convenience constructors for backward compatibility.
    pub fn eof() -> Self {
        Self::Eof { offset: None }
    }
    pub fn invalid_input() -> Self {
        Self::InvalidInput { offset: None }
    }
    pub fn invalid_int() -> Self {
        Self::InvalidInt { offset: None }
    }
    pub fn eof_at(offset: usize) -> Self {
        Self::Eof {
            offset: Some(offset),
        }
    }
    pub fn invalid_input_at(offset: usize) -> Self {
        Self::InvalidInput {
            offset: Some(offset),
        }
    }
    pub fn invalid_int_at(offset: usize) -> Self {
        Self::InvalidInt {
            offset: Some(offset),
        }
    }
}

#[derive(Debug, Error)]
pub enum DecoderError {
    #[error("illegal character")]
    IllegalCharacter,
    #[error("malformed bytes")]
    MalformedBytes,
    #[error("inconsistent length")]
    InconsistentLength,
}

#[derive(Debug, Error)]
pub enum DataError {
    #[error("out of bounds at offset {offset}", offset = .offset.unwrap_or(0))]
    OutOfBounds { offset: Option<usize> },
    #[error("invalid input")]
    InvalidInput,
}

impl DataError {
    pub fn out_of_bounds() -> Self {
        Self::OutOfBounds { offset: None }
    }
    pub fn out_of_bounds_at(offset: usize) -> Self {
        Self::OutOfBounds {
            offset: Some(offset),
        }
    }
}

#[derive(Debug, Error)]
pub enum ByteCodeError {
    #[error("unexpected end of input")]
    Eof,
    #[error("not found")]
    NotFound,
    #[error("invalid input at byte offset {offset}", offset = .offset.unwrap_or(0))]
    InvalidInput { offset: Option<usize> },
    #[error("unknown object type")]
    UnknownObjectType,
    #[error("unknown operand")]
    UnknownOperand,
    #[error("unsupported Tcl bytecode version {major}.{minor}")]
    UnsupportedTclVersion { major: i32, minor: i32 },
}

impl ByteCodeError {
    pub fn invalid_input() -> Self {
        Self::InvalidInput { offset: None }
    }
    pub fn invalid_input_at(offset: usize) -> Self {
        Self::InvalidInput {
            offset: Some(offset),
        }
    }
}

#[derive(Debug, Error)]
pub enum TcldError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Scanner(#[from] CodeScannerError),
    #[error(transparent)]
    Decoder(#[from] DecoderError),
    #[error(transparent)]
    Data(#[from] DataError),
    #[error(transparent)]
    ByteCode(#[from] ByteCodeError),
    #[error("{context}: {source}")]
    WithContext {
        context: String,
        #[source]
        source: Box<TcldError>,
    },
    #[error("{0}")]
    Other(String),
}

impl TcldError {
    /// Wrap this error with additional context (e.g. file path, phase name).
    pub fn with_context(self, ctx: impl Into<String>) -> Self {
        TcldError::WithContext {
            context: ctx.into(),
            source: Box::new(self),
        }
    }
}

pub type Result<T> = std::result::Result<T, TcldError>;
