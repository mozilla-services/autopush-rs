use std::{cell::RefCell, rc::Rc, sync::mpsc};

use actix_web::{dev::ServiceRequest, web::Data};
use uuid::Uuid;

use autoconnect_settings::{options::ServerOptions, Settings};
use autoconnect_web::{
    broadcast::BroadcastSubs,
    client::{Client, WebPushClient},
};
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

/// Called on a new websocket connection
/// copies old autopush::client::Client::new () -> ???
// TODO: Need to figure out what the proper response for this is? UnauthorizedClient?
pub async fn on_websocket_connect(req: &ServiceRequest, uaid: Uuid) -> Result<()> {
    let state = match req.app_data::<Data<ServerOptions>>() {
        Some(v) => v,
        None => return Err(ApcErrorKind::GeneralError("Could not get app data".to_owned()).into()),
    };

    /*
    let (tx, rx) = mpsc::sync_channel(state.max_pending_notification_queue as usize);
     */

    // create the new, unauthorized client.
    /*
    let client = Client::new(rx);
     */

    // get the UserAgent from the request headers
    // Create the ws_client with appropriate channels
    // add client to the ClientRegistry
    //

    Ok(())
}
