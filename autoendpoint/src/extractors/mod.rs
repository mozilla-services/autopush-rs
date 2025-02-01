//! Actix extractors (`FromRequest`). These extractors transform and validate
//! the incoming request data.

pub mod authorization_check;
pub mod message_id;
pub mod new_channel_data;
pub mod notification;
pub mod notification_headers;
pub mod otel_context;
pub mod registration_path_args;
pub mod registration_path_args_with_uaid;
pub mod router_data_input;
pub mod routers;
pub mod subscription;
pub mod token_info;
pub mod user;
