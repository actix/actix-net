use std::io;

use derive_more::Display;

#[derive(Debug, Display)]
pub enum ConnectError {
    /// Failed to resolve the hostname
    #[display(fmt = "Failed resolving hostname: {}", _0)]
    Resolver(Box<dyn std::error::Error>),

    /// No dns records
    #[display(fmt = "No dns records found for the input")]
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
