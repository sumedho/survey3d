use thiserror::Error;

/// Errors produced while parsing user-entered values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("empty input")]
    Empty,
    #[error("'{0}' is not a number")]
    NotANumber(String),
    #[error("minutes must be less than 60 (got {0})")]
    MinutesOutOfRange(String),
    #[error("seconds must be less than 60 (got {0})")]
    SecondsOutOfRange(String),
    #[error("expected {expected} values, got {got}")]
    WrongValueCount { expected: &'static str, got: usize },
    #[error("invalid quadrant bearing '{0}'")]
    InvalidQuadrant(String),
    #[error("{0}")]
    Invalid(String),
}
