use actix_web::HttpResponse;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route() -> HttpResponse {
    HttpResponse::Ok().finish()
}
