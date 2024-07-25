use super::error::StubError;
use super::settings::StubServerSettings;
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

    /// Reply to the "client" based on the the type of app_id they specified.
    pub async fn call(&self, settings: &StubServerSettings) -> Result<(), RouterError> {
        if !self.success {
            return Err(self
                .err
                .clone()
                .unwrap_or(StubError::General(settings.error.clone()))
                .into());
        }

        Ok(())
    }
}
