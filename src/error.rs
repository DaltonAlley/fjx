use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub(crate) struct Error {
    code: u8,
    message: String,
    status: Option<u16>,
}

impl Error {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new(2, message)
    }

    pub(crate) fn context(message: impl Into<String>) -> Self {
        Self::new(3, message)
    }

    pub(crate) fn network(message: impl Into<String>) -> Self {
        Self::new(4, message)
    }

    pub(crate) fn api(message: impl Into<String>) -> Self {
        Self::new(5, message)
    }

    /// An API error that keeps the response status, so a command can explain
    /// a missing route without parsing the message.
    pub(crate) fn http(status: u16, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            ..Self::api(message)
        }
    }

    pub(crate) const fn http_status(&self) -> Option<u16> {
        self.status
    }

    pub(crate) fn safety(message: impl Into<String>) -> Self {
        Self::new(6, message)
    }

    pub(crate) fn data(message: impl Into<String>) -> Self {
        Self::new(7, message)
    }

    pub(crate) fn local(message: impl Into<String>) -> Self {
        Self::new(8, message)
    }

    pub(crate) const fn code(&self) -> u8 {
        self.code
    }

    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            status: None,
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
