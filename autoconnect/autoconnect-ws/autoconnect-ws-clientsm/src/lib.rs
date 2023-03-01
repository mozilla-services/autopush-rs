use std::{cell::RefCell, rc::Rc};

use actix_web::{dev::ServiceRequest, web::Data};
use uuid::Uuid;

use autoconnect_settings::options::ServerOptions;
use autoconnect_web::broadcast::BroadcastSubs;
use autopush_common::errors::{ApcErrorKind, Result};

/// Client state machine
/// handles ServerNotifications queued from ClientRegistry

enum ClientStates {
    UnauthorizedClient {
        user_agent: String,
        broadcast_subs: Rc<RefCell<BroadcastSubs>>,
    },
    AuthorizedClient {},
}
