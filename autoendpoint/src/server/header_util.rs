//! Utilities for working with headers

use actix_web::HttpRequest;

/// Get a header from the request
pub fn get_header<'r>(req: &'r HttpRequest, header: &str) -> Option<&'r str> {
    req.headers().get(header).and_then(|h| h.to_str().ok())
}

/// Get an owned copy of a header from the request
pub fn get_owned_header(req: &HttpRequest, header: &str) -> Option<String> {
    get_header(req, header).map(str::to_string)
}
