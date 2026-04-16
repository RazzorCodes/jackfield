use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum JackfieldError {
    NotRegistered,
    ChannelClosed,
    Custom(String),
}

impl fmt::Display for JackfieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegistered => write!(f, "Endpoint not registered with a bus"),
            Self::ChannelClosed => write!(f, "Bus channel has been closed"),
            Self::Custom(msg) => write!(f, "Bus error: {}", msg),
        }
    }
}

impl Error for JackfieldError {}
