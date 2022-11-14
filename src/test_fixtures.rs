pub enum Request {
    A,
    B,
    Shutdown,
}

#[derive(Debug, PartialEq)]
pub enum Response {
    A,
    B,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<futures::channel::mpsc::SendError> for Error {
    fn from(e: futures::channel::mpsc::SendError) -> Self {
        Self(format!("{e:?}"))
    }
}
