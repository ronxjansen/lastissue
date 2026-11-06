//! Error type. Every error is rendered as an RFC 5322-style header block on
//! stderr and maps to a fixed exit code.

use std::fmt;

pub const EXIT_USAGE: i32 = 1;
pub const EXIT_NOT_FOUND: i32 = 2;
pub const EXIT_AMBIGUOUS: i32 = 3;
pub const EXIT_VALIDATION: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: i32,
    pub kind: String,
    pub fields: Vec<(String, String)>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    fn new(code: i32, kind: &str) -> Self {
        Error {
            code,
            kind: kind.to_string(),
            fields: Vec::new(),
        }
    }

    pub fn usage(kind: &str) -> Self {
        Self::new(EXIT_USAGE, kind)
    }

    pub fn not_found(kind: &str) -> Self {
        Self::new(EXIT_NOT_FOUND, kind)
    }

    pub fn ambiguous(candidates: Vec<String>) -> Self {
        Self::ambiguous_as("ambiguous-id", candidates)
    }

    pub fn ambiguous_as(kind: &str, candidates: Vec<String>) -> Self {
        Self::new(EXIT_AMBIGUOUS, kind).with("Candidates", candidates.join(", "))
    }

    pub fn validation(kind: &str) -> Self {
        Self::new(EXIT_VALIDATION, kind)
    }

    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.fields.push((key.to_string(), value.into()));
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Error: {}", self.kind)?;
        for (k, v) in &self.fields {
            writeln!(f, "{k}: {}", v.replace('\n', " "))?;
        }
        Ok(())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::usage("io").with("Message", e.to_string())
    }
}
