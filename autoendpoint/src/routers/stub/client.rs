use super::error::StubError;
use crate::routers::RouterError;

pub struct StubClient {
    success: bool,
    err: Option<StubError>,
}

impl StubClient {
    /// Always succeeds
    pub fn success() -> Self {
        Self {
            success: true,
            err: None,
        }
    }

    pub fn error(err: StubError) -> Self {
        Self {
            success: false,
            err: Some(err),
        }
    }

    /// Send the message data to Test
    pub async fn call(&self) -> Result<(), RouterError> {
        if !self.success {
            return Err(self
                .err
                .clone()
                .unwrap_or(StubError::General("AAAH!".to_owned()))
                .into());
        }

        Ok(())
    }
}
