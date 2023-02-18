pub enum Error {
    /// Worker error
    WorkerError(String),
}

impl From<worker::Error> for Error {
    fn from(e: worker::Error) -> Self {
        Error::WorkerError(e.to_string())
    }
}
