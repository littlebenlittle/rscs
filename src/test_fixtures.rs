pub enum Request {
    A,
    B,
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
