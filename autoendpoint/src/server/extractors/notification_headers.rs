use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::server::headers::crypto_key::CryptoKeyHeader;
use crate::server::headers::util::{get_header, get_owned_header};
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use lazy_static::lazy_static;
use regex::Regex;
use std::cmp::min;
use validator::Validate;
use validator_derive::Validate;

lazy_static! {
    static ref VALID_BASE64_URL: Regex = Regex::new(r"^[0-9A-Za-z\-_]+=*$").unwrap();
}

const MAX_TTL: i64 = 60 * 60 * 24 * 60;

/// Extractor and validator for notification headers
#[derive(Debug, Validate)]
pub struct NotificationHeaders {
    // TTL is a signed value so that validation can catch negative inputs
    #[validate(range(min = 0, message = "TTL must be greater than 0", code = "114"))]
    pub ttl: Option<i64>,

    #[validate(
        length(
            max = 32,
            message = "Topic must be no greater than 32 characters",
            code = "113"
        ),
        regex(
            path = "VALID_BASE64_URL",
            message = "Topic must be URL and Filename safe Base64 alphabet",
            code = "113"
        )
    )]
    pub topic: Option<String>,

    // These fields are validated separately, because the validation is complex
    // and based upon the content encoding
    pub content_encoding: Option<String>,
    pub encryption: Option<String>,
    pub encryption_key: Option<String>,
    pub crypto_key: Option<String>,
}

impl FromRequest for NotificationHeaders {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, payload: &mut Payload<PayloadStream>) -> Self::Future {
        // Collect raw headers
        let ttl = get_header(req, "ttl")
            .and_then(|ttl| ttl.parse().ok())
            // Enforce a maximum TTL, but don't error
            .map(|ttl| min(ttl, MAX_TTL));
        let topic = get_owned_header(req, "topic");
        let content_encoding = get_owned_header(req, "content-encoding");
        let encryption = get_owned_header(req, "encryption");
        let encryption_key = get_owned_header(req, "encryption-key");
        let crypto_key = get_owned_header(req, "crypto-key");

        let headers = NotificationHeaders {
            ttl,
            topic,
            content_encoding,
            encryption,
            encryption_key,
            crypto_key,
        };

        // Validate encryption if there is a message body
        if !matches!(payload, Payload::None) {
            match headers.validate_encryption() {
                Ok(_) => {}
                Err(e) => return future::err(e),
            }
        }

        // Validate the other headers
        match headers.validate() {
            Ok(_) => future::ok(headers),
            Err(e) => future::err(ApiError::from(e)),
        }
    }
}

impl NotificationHeaders {
    /// Validate the encryption headers according to the various WebPush
    /// standard versions
    fn validate_encryption(&self) -> ApiResult<()> {
        let content_encoding = self.content_encoding.as_deref().ok_or_else(|| {
            ApiErrorKind::InvalidEncryption("Missing Content-Encoding header".to_string())
        })?;

        match content_encoding {
            "aesgcm128" => self.validate_encryption_01_rules()?,
            "aesgcm" => self.validate_encryption_04_rules()?,
            "aes128gcm" => self.validate_encryption_06_rules()?,
            _ => {
                return Err(ApiErrorKind::InvalidEncryption(
                    "Unknown Content-Encoding header".to_string(),
                )
                .into());
            }
        }

        Ok(())
    }

    /// Validates encryption headers according to
    /// draft-ietf-webpush-encryption-01
    fn validate_encryption_01_rules(&self) -> ApiResult<()> {
        Self::assert_base64_item_exists("Encryption", self.encryption.as_deref(), "salt")?;
        Self::assert_base64_item_exists("Encryption-Key", self.encryption_key.as_deref(), "dh")?;
        Self::assert_not_exists("aesgcm128 Crypto-Key", self.crypto_key.as_deref(), "dh")?;

        Ok(())
    }

    /// Validates encryption headers according to
    /// draft-ietf-webpush-encryption-04
    fn validate_encryption_04_rules(&self) -> ApiResult<()> {
        Self::assert_base64_item_exists("Encryption", self.encryption.as_deref(), "salt")?;

        if self.encryption_key.is_some() {
            return Err(ApiErrorKind::InvalidEncryption(
                "Encryption-Key header is not valid for webpush draft 02 or later".to_string(),
            )
            .into());
        }

        if self.crypto_key.is_some() {
            Self::assert_base64_item_exists("Crypto-Key", self.crypto_key.as_deref(), "dh")?;
        }

        Ok(())
    }

    /// Validates encryption headers according to
    /// draft-ietf-webpush-encryption-06
    fn validate_encryption_06_rules(&self) -> ApiResult<()> {
        Self::assert_not_exists("aes128gcm Encryption", self.encryption.as_deref(), "salt")?;
        Self::assert_not_exists("aes128gcm Crypto-Key", self.crypto_key.as_deref(), "dh")?;

        Ok(())
    }

    /// Assert that the given key exists in the header and is valid base64.
    fn assert_base64_item_exists(
        header_name: &str,
        header: Option<&str>,
        key: &str,
    ) -> ApiResult<()> {
        let header = header.ok_or_else(|| {
            ApiErrorKind::InvalidEncryption(format!("Missing {} header", header_name))
        })?;
        let header_data = CryptoKeyHeader::parse(header).ok_or_else(|| {
            ApiErrorKind::InvalidEncryption(format!("Invalid {} header", header_name))
        })?;
        let salt = header_data.get_by_key(key).ok_or_else(|| {
            ApiErrorKind::InvalidEncryption(format!(
                "Missing {} value in {} header",
                key, header_name
            ))
        })?;

        if !VALID_BASE64_URL.is_match(salt) {
            return Err(ApiErrorKind::InvalidEncryption(format!(
                "Invalid {} value in {} header",
                key, header_name
            ))
            .into());
        }

        Ok(())
    }

    /// Assert that the given key does not exist in the header.
    fn assert_not_exists(header_name: &str, header: Option<&str>, key: &str) -> ApiResult<()> {
        let header = match header {
            Some(header) => header,
            None => return Ok(()),
        };

        let header_data = CryptoKeyHeader::parse(header).ok_or_else(|| {
            ApiErrorKind::InvalidEncryption(format!("Invalid {} header", header_name))
        })?;

        if header_data.get_by_key(key).is_some() {
            return Err(ApiErrorKind::InvalidEncryption(format!(
                "Do not include '{}' header in {} header",
                key, header_name
            ))
            .into());
        }

        Ok(())
    }
}
