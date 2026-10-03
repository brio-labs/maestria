use std::error::Error;

/// Unwrap a `Result::Err` or return a test error with `message`.
pub fn require_error<T, E>(result: Result<T, E>, message: &str) -> Result<E, Box<dyn Error>> {
    match result {
        Ok(_) => Err(std::io::Error::other(message).into()),
        Err(error) => Ok(error),
    }
}
