/// Handle incoming notifications from endpoints.
///

use actix_web::{
    FromRequest, HttpRequest, HttpResponse,
    dev::{Payload, PayloadStream},
    web::{Data, Json},
};
use reqwest::StatusCode;
use serde_json::json;
use uuid::Uuid;

use autopush_common::error::{ApiError, ApiErrorKind};


use autoconnect_settings::options::ServerOptions;

pub struct InterNodeArgs {
    pub uaid: Uuid
}

impl FromRequest for InterNodeArgs {
    type Error  = ApiError;

}

pub struct InterNode {

}

impl InterNode {
    pub fn put(state: Data<ServerOptions>) -> HttpResponse {

    }
}
