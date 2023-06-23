use actix_http::ws::CloseReason;
use async_trait::async_trait;
use mockall::automock;

use autoconnect_common::protocol::ServerMessage;

use crate::error::WSError;

/// Trait wrapping [`actix_ws::Session`] so it can be replaced by e.g. a mock.
///
/// This takes `ServerMessage`s and returns `WSErrors` to ease integration and
/// usage via mocking. `Binary` WebSocket messages aren't supported (as we
/// don't use them).
#[async_trait]
#[automock]
pub trait Session {
    /// See [`actix_ws::Session::text`]
    async fn text(&mut self, msg: ServerMessage) -> Result<(), WSError>;

    /// See [`actix_ws::Session::ping`]
    async fn ping(&mut self, msg: &[u8]) -> Result<(), WSError>;

    /// See [`actix_ws::Session::pong`]
    async fn pong(&mut self, msg: &[u8]) -> Result<(), WSError>;

    /// See [`actix_ws::Session::close`]
    async fn close(mut self, reason: Option<CloseReason>) -> Result<(), WSError>;
}

/// Implements our `Session` wrapping [`actix_ws::Session`]
#[derive(Clone)]
pub struct SessionImpl {
    inner: actix_ws::Session,
}

impl SessionImpl {
    pub fn new(inner: actix_ws::Session) -> Self {
        SessionImpl { inner }
    }
}

#[async_trait]
impl Session for SessionImpl {
    async fn text(&mut self, msg: ServerMessage) -> Result<(), WSError> {
        Ok(self.inner.text(msg.to_json()?).await?)
    }

    async fn ping(&mut self, msg: &[u8]) -> Result<(), WSError> {
        Ok(self.inner.ping(msg).await?)
    }

    async fn pong(&mut self, msg: &[u8]) -> Result<(), WSError> {
        Ok(self.inner.pong(msg).await?)
    }

    async fn close(mut self, reason: Option<CloseReason>) -> Result<(), WSError> {
        Ok(self.inner.close(reason).await?)
    }
}
