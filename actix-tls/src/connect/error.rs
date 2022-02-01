use std::{error::Error, fmt, io};

/// Errors that can result from using a connector service.
#[derive(Debug)]
pub enum ConnectError {
    /// Failed to resolve the hostname.
    Resolver(Box<dyn std::error::Error>),

    /// No DNS records.
    NoRecords,

    /// Invalid input.
    InvalidInput,

    /// Unresolved host name.
    Unresolved,

    /// Connection IO error.
    Io(io::Error),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRecords => f.write_str("No DNS records found for the input"),
            Self::InvalidInput => f.write_str("Invalid input"),
            Self::Unresolved => {
                f.write_str("Connector received `Connect` method with unresolved host")
            }
            Self::Resolver(_) => f.write_str("Failed to resolve hostname"),
            Self::Io(_) => f.write_str("I/O error"),
        }
    }
}

impl Error for ConnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resolver(err) => Some(&**err),
            Self::Io(err) => Some(err),
            Self::NoRecords | Self::InvalidInput | Self::Unresolved => None,
        }
    }
}
