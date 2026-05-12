use std::fmt;

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug)]
pub enum BrowserError {
    Io(std::io::Error),
    Message(String),
}

impl BrowserError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Message(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for BrowserError {}

impl From<std::io::Error> for BrowserError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
