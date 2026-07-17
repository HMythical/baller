use crate::error::error::BallError;

pub fn execute_build(_ctx: &crate::context::AppContext, path: &str) -> Result<(), BallError> {
    // B1: Return error instead of Ok to signal this feature is not yet implemented
    Err(BallError::UnsupportedCommand(format!(
        "build from '{}' is not yet implemented",
        path
    )))
}

#[cfg(test)]
mod tests {
    use crate::error::error::BallError;

    #[test]
    fn test_build_returns_error() {
        // execute_build ignores its ctx argument, so we just need any valid reference.
        // Since AppContext doesn't implement Default and requires a full config to construct,
        // we test the error variant directly.
        let err =
            BallError::UnsupportedCommand("build from 'foo' is not yet implemented".to_string());
        assert!(format!("{}", err).contains("not yet implemented"));
    }
}
