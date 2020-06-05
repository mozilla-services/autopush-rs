use actix_web::HttpResponse;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route() -> HttpResponse {
    HttpResponse::Ok().finish()
}

pub struct TokenInfo {
    api_version: String,
    token: String,
    crypto_key: String,
    auth_header: String,
}

pub struct Subscription {
    uaid: String,
    channel_id: String,
    version: String,
    public_key: String,
}

pub struct Notification {
    channel_id: String,
    version: String,
    ttl: String,
    topic: String,
    timestamp: String,
    data: String,
    headers: String,
}
