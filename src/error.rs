use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub(crate) struct Error {
    code: u8,
    message: String,
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
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
