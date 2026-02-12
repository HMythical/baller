use std::fmt;

#[derive(Debug)]
pub enum BallError {
    UnsupportedOs(String),
    UnsupportedCommand(String),
}

impl fmt::Display for BallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BallError::UnsupportedOs(os) => write!(f, "the following OS is unsupported: {}\n\t please use Windows or Linux", os),

            BallError::UnsupportedCommand(command) => write!(f, "the following command does not exist: {}\n\t run 'baller help' for more info", command),
        }
    }
}
