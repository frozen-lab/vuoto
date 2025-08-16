use turbocache::TurboError;

pub(crate) type InternalResult<T> = Result<T, InternalError>;

#[derive(Debug)]
pub(crate) enum InternalError {
    IO(String),
    Unknown(String),
}

impl From<std::io::Error> for InternalError {
    fn from(err: std::io::Error) -> Self {
        InternalError::IO(format!("{}", err))
    }
}

impl From<TurboError> for InternalError {
    fn from(err: TurboError) -> Self {
        match err {
            TurboError::Io(e) => InternalError::IO(format!("{}", e)),
            _ => InternalError::Unknown("".into()),
        }
    }
}
