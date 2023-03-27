/*
use std::{cell::RefCell, rc::Rc};
use autoconnect_web::broadcast::BroadcastSubs;

/// Client state machine
/// handles ServerNotifications queued from ClientRegistry

enum ClientStates {
    UnauthorizedClient {
        user_agent: String,
        broadcast_subs: Rc<RefCell<BroadcastSubs>>,
    },
    AuthorizedClient {},
}

*/
