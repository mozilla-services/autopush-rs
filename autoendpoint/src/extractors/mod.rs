//! Actix extractors (`FromRequest`). These extractors transform and validate
//! the incoming request data.

pub mod notification;
pub mod notification_headers;
pub mod registration_path_args;
pub mod router_data_input;
pub mod routers;
pub mod subscription;
pub mod token_info;
pub mod user;
