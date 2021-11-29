use std::{error::Error, io};

use derive_more::Display;

/// Errors that can result from using a connector service.
#[derive(Debug, Display)]
pub enum ConnectError {
    /// Failed to resolve the hostname
    #[display(fmt = "Failed resolving hostname")]
    Resolver(Box<dyn std::error::Error>),

    /// No DNS records
    #[display(fmt = "No DNS records found for the input")]
    NoRecords,

    /// Invalid input
    InvalidInput,

    /// Unresolved host name
    #[display(fmt = "Connector received `Connect` method with unresolved host")]
    Unresolved,

    /// Connection IO error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
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
