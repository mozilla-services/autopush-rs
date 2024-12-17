use crate::routers::common::message_size_check;
use crate::routers::fcm::error::FcmError;
use crate::routers::fcm::settings::{FcmServerCredential, FcmSettings};
use crate::routers::RouterError;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use url::Url;
use yup_oauth2::authenticator::DefaultAuthenticator;
use yup_oauth2::{ServiceAccountAuthenticator, ServiceAccountKey};

const OAUTH_SCOPES: &[&str] = &["https://www.googleapis.com/auth/firebase.messaging"];

/// Holds application-specific Firebase data and authentication. This client
/// handles sending notifications to Firebase.
pub struct FcmClient {
    endpoint: Url,
    timeout: Duration,
    max_data: usize,
    authenticator: Option<DefaultAuthenticator>,
    http_client: reqwest::Client,
}

impl FcmClient {
    /// Create an `FcmClient` using the provided credential
    pub async fn new(
        settings: &FcmSettings,
        server_credential: FcmServerCredential,
        http: reqwest::Client,
    ) -> std::io::Result<Self> {
        // `map`ping off of `serde_json::from_str` gets hairy and weird, requiring
        // async blocks and a number of other specialty items. Doing a very stupid
        // json detection does not. FCM keys are serialized JSON constructs.
        // These are both set in the settings and come from the `credentials` value.
        let auth = if server_credential.server_access_token.contains('{') {
            trace!(
                "Reading credential for {} from string...",
                &server_credential.project_id
            );
            let key_data =
                serde_json::from_str::<ServiceAccountKey>(&server_credential.server_access_token)?;
            Some(
                ServiceAccountAuthenticator::builder(key_data)
                    .build()
                    .await?,
            )
        } else {
            // check to see if this is a path to a file, and read in the credentials.
            if Path::new(&server_credential.server_access_token).exists() {
                warn!(
                    "Reading credential for {} from file...",
                    &server_credential.project_id
                );
                let content = std::fs::read_to_string(&server_credential.server_access_token)?;
                let key_data = serde_json::from_str::<ServiceAccountKey>(&content)?;
                Some(
                    ServiceAccountAuthenticator::builder(key_data)
                        .build()
                        .await?,
                )
            } else {
                trace!("Presuming {} is GCM", &server_credential.project_id);
                None
            }
        };
        Ok(FcmClient {
            endpoint: settings
                .base_url
                .join(&format!(
                    "v1/projects/{}/messages:send",
                    server_credential.project_id
                ))
                .expect("Project ID is not URL-safe"),
            timeout: Duration::from_secs(settings.timeout as u64),
            max_data: settings.max_data,
            authenticator: auth,
            http_client: http,
        })
    }

    /// Send the message data to FCM
    pub async fn send(
        &self,
        data: HashMap<&'static str, String>,
        routing_token: String,
        ttl: u64,
    ) -> Result<(), RouterError> {
        // Check the payload size. FCM only cares about the `data` field when
        // checking size.
        let data_json = serde_json::to_string(&data).unwrap();
        message_size_check(data_json.as_bytes(), self.max_data)?;

        // Build the FCM message
        let message = serde_json::json!({
            "message": {
                "token": routing_token,
                "android": {
                    "ttl": format!("{ttl}s"),
                    "data": data
                }
            }
        });

        let server_access_token = self
            .authenticator
            .as_ref()
            .unwrap()
            .token(OAUTH_SCOPES)
            .await
            .map_err(FcmError::OAuthToken)?;
        let token = server_access_token.token().ok_or(FcmError::NoOAuthToken)?;

        // Make the request
        let response = self
            .http_client
            .post(self.endpoint.clone())
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&message)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RouterError::RequestTimeout
                } else {
                    RouterError::Connect(e)
                }
            })?;

        // Handle error
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            let raw_data = response
                .bytes()
                .await
                .map_err(FcmError::DeserializeResponse)?;
            if raw_data.is_empty() {
                warn!("Empty FCM response [{status}]");
                return Err(FcmError::EmptyResponse(status).into());
            }
            let data: FcmResponse = serde_json::from_slice(&raw_data).map_err(|e| {
                let s = String::from_utf8(raw_data.to_vec()).unwrap_or_else(|e| e.to_string());
                warn!("Invalid FCM response [{status}] \"{s}\"");
                FcmError::InvalidResponse(e, s, status)
            })?;

            // we only ever send one.
            return Err(match (status, data.error) {
                (StatusCode::UNAUTHORIZED, _) => RouterError::Authentication,
                (StatusCode::NOT_FOUND, _) => RouterError::NotFound,
                (_, Some(error)) => {
                    info!("🌉Bridge Error: {:?}, {:?}", error.message, &self.endpoint);
                    FcmError::Upstream {
                        error_code: error.status, // Note: this is the FCM error status enum value
                        message: error.message,
                    }
                    .into()
                }
                // In this case, we've gotten an error, but FCM hasn't returned a body.
                // (This may happen in the case where FCM terminates the connection abruptly
                // or a similar event.) Treat that as an INTERNAL error.
                (_, None) => {
                    warn!(
                        "🌉Unknown Bridge Error: {:?}, <{:?}>, [{:?}]",
                        status.to_string(),
                        &self.endpoint,
                        raw_data,
                    );
                    FcmError::Upstream {
                        error_code: "UNKNOWN".to_string(),
                        message: format!("Unknown reason: {:?}", status.to_string()),
                    }
                }
                .into(),
            });
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct FcmResponse {
    error: Option<FcmErrorResponse>,
}

/// Response message from FCM in the case of an error.
#[derive(Deserialize)]
struct FcmErrorResponse {
    /// The ErrorCode enum as string from https://firebase.google.com/docs/reference/fcm/rest/v1/ErrorCode
    status: String,
    message: String,
}

#[cfg(test)]
pub mod tests {
    use crate::routers::fcm::client::FcmClient;
    use crate::routers::fcm::error::FcmError;
    use crate::routers::fcm::settings::{FcmServerCredential, FcmSettings};
    use crate::routers::RouterError;
    use std::collections::HashMap;
    use url::Url;

    pub const PROJECT_ID: &str = "yup-test-243420";
    const ACCESS_TOKEN: &str = "ya29.c.ElouBywiys0LyNaZoLPJcp1Fdi2KjFMxzvYKLXkTdvM-rDfqKlvEq6PiMhGoGHx97t5FAvz3eb_ahdwlBjSStxHtDVQB4ZPRJQ_EOi-iS7PnayahU2S9Jp8S6rk";
    pub const GCM_PROJECT_ID: &str = "valid_gcm_access_token";

    /// Write service data to a temporary file
    pub fn make_service_key(server: &mockito::ServerGuard) -> String {
        // Taken from the yup-oauth2 tests
        serde_json::json!({
            "type": "service_account",
            "project_id": PROJECT_ID,
            "private_key_id": "26de294916614a5ebdf7a065307ed3ea9941902b",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQDemmylrvp1KcOn\n9yTAVVKPpnpYznvBvcAU8Qjwr2fSKylpn7FQI54wCk5VJVom0jHpAmhxDmNiP8yv\nHaqsef+87Oc0n1yZ71/IbeRcHZc2OBB33/LCFqf272kThyJo3qspEqhuAw0e8neg\nLQb4jpm9PsqR8IjOoAtXQSu3j0zkXemMYFy93PWHjVpPEUX16NGfsWH7oxspBHOk\n9JPGJL8VJdbiAoDSDgF0y9RjJY5I52UeHNhMsAkTYs6mIG4kKXt2+T9tAyHw8aho\nwmuytQAfydTflTfTG8abRtliF3nil2taAc5VB07dP1b4dVYy/9r6M8Z0z4XM7aP+\nNdn2TKm3AgMBAAECggEAWi54nqTlXcr2M5l535uRb5Xz0f+Q/pv3ceR2iT+ekXQf\n+mUSShOr9e1u76rKu5iDVNE/a7H3DGopa7ZamzZvp2PYhSacttZV2RbAIZtxU6th\n7JajPAM+t9klGh6wj4jKEcE30B3XVnbHhPJI9TCcUyFZoscuPXt0LLy/z8Uz0v4B\nd5JARwyxDMb53VXwukQ8nNY2jP7WtUig6zwE5lWBPFMbi8GwGkeGZOruAK5sPPwY\nGBAlfofKANI7xKx9UXhRwisB4+/XI1L0Q6xJySv9P+IAhDUI6z6kxR+WkyT/YpG3\nX9gSZJc7qEaxTIuDjtep9GTaoEqiGntjaFBRKoe+VQKBgQDzM1+Ii+REQqrGlUJo\nx7KiVNAIY/zggu866VyziU6h5wjpsoW+2Npv6Dv7nWvsvFodrwe50Y3IzKtquIal\nVd8aa50E72JNImtK/o5Nx6xK0VySjHX6cyKENxHRDnBmNfbALRM+vbD9zMD0lz2q\nmns/RwRGq3/98EqxP+nHgHSr9QKBgQDqUYsFAAfvfT4I75Glc9svRv8IsaemOm07\nW1LCwPnj1MWOhsTxpNF23YmCBupZGZPSBFQobgmHVjQ3AIo6I2ioV6A+G2Xq/JCF\nmzfbvZfqtbbd+nVgF9Jr1Ic5T4thQhAvDHGUN77BpjEqZCQLAnUWJx9x7e2xvuBl\n1A6XDwH/ewKBgQDv4hVyNyIR3nxaYjFd7tQZYHTOQenVffEAd9wzTtVbxuo4sRlR\nNM7JIRXBSvaATQzKSLHjLHqgvJi8LITLIlds1QbNLl4U3UVddJbiy3f7WGTqPFfG\nkLhUF4mgXpCpkMLxrcRU14Bz5vnQiDmQRM4ajS7/kfwue00BZpxuZxst3QKBgQCI\nRI3FhaQXyc0m4zPfdYYVc4NjqfVmfXoC1/REYHey4I1XetbT9Nb/+ow6ew0UbgSC\nUZQjwwJ1m1NYXU8FyovVwsfk9ogJ5YGiwYb1msfbbnv/keVq0c/Ed9+AG9th30qM\nIf93hAfClITpMz2mzXIMRQpLdmQSR4A2l+E4RjkSOwKBgQCB78AyIdIHSkDAnCxz\nupJjhxEhtQ88uoADxRoEga7H/2OFmmPsqfytU4+TWIdal4K+nBCBWRvAX1cU47vH\nJOlSOZI0gRKe0O4bRBQc8GXJn/ubhYSxI02IgkdGrIKpOb5GG10m85ZvqsXw3bKn\nRVHMD0ObF5iORjZUqD0yRitAdg==\n-----END PRIVATE KEY-----\n",
            "client_email": "yup-test-sa-1@yup-test-243420.iam.gserviceaccount.com",
            "client_id": "102851967901799660408",
            "auth_uri": "https://accounts.google.com/o/oauth2/auth",
            "token_uri": server.url() + "/token",
            "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
            "client_x509_cert_url": "https://www.googleapis.com/robot/v1/metadata/x509/yup-test-sa-1%40yup-test-243420.iam.gserviceaccount.com"
        }).to_string()
    }

    /// Mock the OAuth token endpoint to provide the access token
    pub async fn mock_token_endpoint(server: &mut mockito::ServerGuard) -> mockito::Mock {
        server
            .mock("POST", "/token")
            .with_body(
                serde_json::json!({
                    "access_token": ACCESS_TOKEN,
                    "expires_in": 3600,
                    "token_type": "Bearer"
                })
                .to_string(),
            )
            .create_async()
            .await
    }

    /// Start building a mock for the FCM endpoint
    pub fn mock_fcm_endpoint_builder(server: &mut mockito::ServerGuard, id: &str) -> mockito::Mock {
        server.mock("POST", format!("/v1/projects/{id}/messages:send").as_str())
    }

    /// Make a FcmClient from the service auth data
    async fn make_client(
        server: &mockito::ServerGuard,
        credential: FcmServerCredential,
    ) -> FcmClient {
        FcmClient::new(
            &FcmSettings {
                base_url: Url::parse(&server.url()).unwrap(),
                server_credentials: serde_json::json!(credential).to_string(),
                ..Default::default()
            },
            credential,
            reqwest::Client::new(),
        )
        .await
        .unwrap()
    }

    /// The FCM client uses the access token and parameters to build the
    /// expected FCM request.
    #[tokio::test]
    async fn sends_correct_fcm_request() {
        let mut server = mockito::Server::new_async().await;

        let client = make_client(
            &server,
            FcmServerCredential {
                project_id: PROJECT_ID.to_owned(),
                is_gcm: None,
                server_access_token: make_service_key(&server),
            },
        )
        .await;
        let _token_mock = mock_token_endpoint(&mut server).await;
        let fcm_mock = mock_fcm_endpoint_builder(&mut server, PROJECT_ID)
            .match_header("Authorization", format!("Bearer {ACCESS_TOKEN}").as_str())
            .match_header("Content-Type", "application/json")
            .match_body(r#"{"message":{"android":{"data":{"is_test":"true"},"ttl":"42s"},"token":"test-token"}}"#)
            .create();

        let mut data = HashMap::new();
        data.insert("is_test", "true".to_string());

        let result = client.send(data, "test-token".to_string(), 42).await;
        assert!(result.is_ok(), "result = {result:?}");
        fcm_mock.assert();
    }

    /// Authorization errors are handled
    #[tokio::test]
    async fn unauthorized() {
        let mut server = mockito::Server::new_async().await;

        let client = make_client(
            &server,
            FcmServerCredential {
                project_id: PROJECT_ID.to_owned(),
                is_gcm: None,
                server_access_token: make_service_key(&server),
            },
        )
        .await;
        let _token_mock = mock_token_endpoint(&mut server).await;
        let _fcm_mock = mock_fcm_endpoint_builder(&mut server, PROJECT_ID)
            .with_status(401)
            .with_body(r#"{"error":{"status":"UNAUTHENTICATED","message":"test-message"}}"#)
            .create_async()
            .await;

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), RouterError::Authentication),
            "result = {result:?}"
        );
    }

    /// 404 errors are handled
    #[tokio::test]
    async fn not_found() {
        let mut server = mockito::Server::new_async().await;

        let client = make_client(
            &server,
            FcmServerCredential {
                project_id: PROJECT_ID.to_owned(),
                is_gcm: None,
                server_access_token: make_service_key(&server),
            },
        )
        .await;
        let _token_mock = mock_token_endpoint(&mut server).await;
        let _fcm_mock = mock_fcm_endpoint_builder(&mut server, PROJECT_ID)
            .with_status(404)
            .with_body(r#"{"error":{"status":"NOT_FOUND","message":"test-message"}}"#)
            .create_async()
            .await;

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), RouterError::NotFound),
            "result = {result:?}"
        );
    }

    /// Unhandled errors (where an error object is returned) are wrapped and returned
    #[tokio::test]
    async fn other_fcm_error() {
        let mut server = mockito::Server::new_async().await;

        let client = make_client(
            &server,
            FcmServerCredential {
                project_id: PROJECT_ID.to_owned(),
                is_gcm: Some(false),
                server_access_token: make_service_key(&server),
            },
        )
        .await;
        let _token_mock = mock_token_endpoint(&mut server).await;
        let _fcm_mock = mock_fcm_endpoint_builder(&mut server, PROJECT_ID)
            .with_status(400)
            .with_body(r#"{"error":{"status":"TEST_ERROR","message":"test-message"}}"#)
            .create_async()
            .await;

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                RouterError::Fcm(FcmError::Upstream{ error_code, message })
                    if error_code == "TEST_ERROR" && message == "test-message"
            ),
            "result = {result:?}"
        );
    }

    /// Unknown errors (where an error object is NOT returned) is handled
    #[tokio::test]
    async fn unknown_fcm_error() {
        let mut server = mockito::Server::new_async().await;

        let client = make_client(
            &server,
            FcmServerCredential {
                project_id: PROJECT_ID.to_owned(),
                is_gcm: Some(true),
                server_access_token: make_service_key(&server),
            },
        )
        .await;
        let _token_mock = mock_token_endpoint(&mut server).await;
        let _fcm_mock = mock_fcm_endpoint_builder(&mut server, PROJECT_ID)
            .with_status(400)
            .with_body("{}")
            .create_async()
            .await;

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                RouterError::Fcm(FcmError::Upstream { error_code, message })
                    if error_code == "UNKNOWN" && message.starts_with("Unknown reason")
            ),
            "result = {result:?}"
        );
    }
}
