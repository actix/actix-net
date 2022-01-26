use actix_utils::derive;
use std::io;

/// Errors that can result from using a connector service.
#[derive(Debug)]
pub enum ConnectError {
    /// Failed to resolve the hostname
    Resolver(Box<dyn std::error::Error>),
    /// No DNS records
    NoRecords,
    /// Invalid input
    InvalidInput,
    /// Unresolved host name
    Unresolved,
    /// Connection IO error
    Io(io::Error),
}

derive::enum_error! {
    match ConnectError |f| {
        NoRecords => "No DNS records found for the input",
        InvalidInput => "Invalid input",
        Unresolved => "Connector received `Connect` method with unresolved host",
        #[source(&**e)] Resolver(e) => "Failed to resolve hostname",
        #[source(e)] Io(e) => return write!(f, "{}", e),
    }
}
