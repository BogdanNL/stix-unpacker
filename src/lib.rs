pub mod archive;
pub mod discovery;
pub mod explode;
pub mod output;

use std::io;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
