use std::fmt;

/// Every error SwiftWin reports. Messages are written for the person sitting in
/// front of the phone: they say what failed and, where possible, what to do.
#[derive(Debug, thiserror::Error)]
pub enum SwiftWinError {
    #[error("no iPhone is connected over USB (is the cable plugged in, and is the Apple Devices app or iTunes installed?)")]
    NoDevice,
    #[error("device {0} is not connected")]
    DeviceNotFound(String),
    #[error("this computer is not trusted by the iPhone: unlock the phone and tap \"Trust\" ({0})")]
    NotTrusted(String),
    #[error("Developer Mode is off: on the iPhone open Settings > Privacy & Security > Developer Mode and turn it on")]
    DeveloperModeOff,
    #[error("{tool} failed: {message}")]
    ExternalTool { tool: String, message: String },
    #[error("{tool} is not installed: {hint}")]
    ToolMissing { tool: String, hint: String },
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("authentication required: {0}")]
    Auth(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl SwiftWinError {
    pub fn other(msg: impl fmt::Display) -> Self {
        SwiftWinError::Other(msg.to_string())
    }
    pub fn protocol(msg: impl fmt::Display) -> Self {
        SwiftWinError::Protocol(msg.to_string())
    }
    pub fn invalid(msg: impl fmt::Display) -> Self {
        SwiftWinError::InvalidInput(msg.to_string())
    }
    pub fn tool(tool: impl Into<String>, message: impl fmt::Display) -> Self {
        SwiftWinError::ExternalTool { tool: tool.into(), message: message.to_string() }
    }
}

impl From<serde_json::Error> for SwiftWinError {
    fn from(e: serde_json::Error) -> Self {
        SwiftWinError::Protocol(format!("JSON: {e}"))
    }
}

pub type Result<T, E = SwiftWinError> = std::result::Result<T, E>;
