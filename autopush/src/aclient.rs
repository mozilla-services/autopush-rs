/// Async based client that is less dependent on a state machine trait
///
/// The previous state machine required manipulating inner workings of
/// `future 0.1` to work correctly. Current, simpler state machine
/// frameworks like `rust_fsm` are too simplistic for what we need,
/// offering  only separate state transition and output direction.
///
/// States are fairly simple in autopush, you can be:
/// Pending Authorization or In A Command Loop. You only ever transition
/// from Pending Authorization to Command Loop, since Errors disconnect
/// the client with an error message.
///
/// The Pending Authorization path is:
/// Client connection => wait TIMEOUT for initial `hello` message type
/// Once the `hello` has been validated and processed, we can drop
/// immediately into the Command loop.
///
/// The Command Loop consists of an infinite loop that awaits the socket
/// times out after

/// TODO: actually build out all of that, based on actix-websocket


use actix::{Actor, StreamHandler};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;


