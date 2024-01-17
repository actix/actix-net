use super::Host;

impl Host for http_0_2::Uri {
    fn hostname(&self) -> &str {
        self.host().unwrap_or("")
    }

    fn port(&self) -> Option<u16> {
        match self.port_u16() {
            Some(port) => Some(port),
            None => scheme_to_port(self.scheme_str()),
        }
    }
}

impl Host for http_1::Uri {
    fn hostname(&self) -> &str {
        self.host().unwrap_or("")
    }

    fn port(&self) -> Option<u16> {
        match self.port_u16() {
            Some(port) => Some(port),
            None => scheme_to_port(self.scheme_str()),
        }
    }
}

// Get port from well-known URL schemes.
fn scheme_to_port(scheme: Option<&str>) -> Option<u16> {
    match scheme {
        // HTTP
        Some("http") => Some(80),
        Some("https") => Some(443),

        // WebSockets
        Some("ws") => Some(80),
        Some("wss") => Some(443),

        // Advanced Message Queuing Protocol (AMQP)
        Some("amqp") => Some(5672),
        Some("amqps") => Some(5671),

        // Message Queuing Telemetry Transport (MQTT)
        Some("mqtt") => Some(1883),
        Some("mqtts") => Some(8883),

        // File Transfer Protocol (FTP)
        Some("ftp") => Some(21),
        Some("ftps") => Some(990),

        // Redis
        Some("redis") => Some(6379),

        // MySQL
        Some("mysql") => Some(3306),

        // PostgreSQL
        Some("postgres") => Some(5432),

        _ => None,
    }
}
