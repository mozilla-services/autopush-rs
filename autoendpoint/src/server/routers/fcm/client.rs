use crate::server::routers::fcm::error::FcmError;
use crate::server::routers::fcm::settings::FcmCredential;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;
use yup_oauth2::authenticator::DefaultAuthenticator;
use yup_oauth2::ServiceAccountAuthenticator;

/// The timeout for FCM requests, in seconds
const TIMEOUT: Duration = Duration::from_secs(3);
const OAUTH_SCOPES: &[&str] = &["https://www.googleapis.com/auth/firebase.messaging"];

/// Holds application-specific Firebase data and authentication. This client
/// handles sending notifications to Firebase.
pub struct FcmClient {
    endpoint: Url,
    auth: DefaultAuthenticator,
    http: reqwest::Client,
}

impl FcmClient {
    /// Create an `FcmClient` using the provided credential
    pub async fn new(
        fcm_url: Url,
        credential: FcmCredential,
        http: reqwest::Client,
    ) -> std::io::Result<Self> {
        let key_data = yup_oauth2::read_service_account_key(&credential.auth_file).await?;
        let auth = ServiceAccountAuthenticator::builder(key_data)
            .build()
            .await?;

        Ok(FcmClient {
            endpoint: fcm_url
                .join(&format!(
                    "v1/projects/{}/messages:send",
                    credential.project_id
                ))
                .expect("Project ID is not URL-safe"),
            auth,
            http,
        })
    }

    /// Send the message data to FCM
    pub async fn send(
        &self,
        data: HashMap<&'static str, String>,
        token: String,
        ttl: usize,
    ) -> Result<(), FcmError> {
        // Build the FCM message
        let message = serde_json::json!({
            "message": {
                "token": token,
                "android": {
                    "ttl": format!("{}s", ttl),
                    "data": data
                }
            }
        });
        let access_token = self.auth.token(OAUTH_SCOPES).await?;

        // Make the request
        let response = self
            .http
            .post(self.endpoint.clone())
            .header("Authorization", format!("Bearer {}", access_token.as_str()))
            .header("Content-Type", "application/json; UTF-8")
            .body(message.to_string())
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    FcmError::FcmRequestTimeout
                } else {
                    FcmError::FcmConnect(e)
                }
            })?;

        // Handle error
        if let Err(e) = response.error_for_status_ref() {
            let data: FcmResponse = response
                .json()
                .await
                .map_err(FcmError::DeserializeResponse)?;

            return Err(match (e.status(), data.error) {
                (Some(StatusCode::UNAUTHORIZED), _) => FcmError::FcmAuthentication,
                (Some(StatusCode::NOT_FOUND), _) => FcmError::FcmNotFound,
                (_, Some(error)) => FcmError::FcmUpstream {
                    status: error.status,
                    message: error.message,
                },
                _ => FcmError::FcmUnknown,
            });
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct FcmResponse {
    error: Option<FcmErrorResponse>,
}

#[derive(Deserialize)]
struct FcmErrorResponse {
    status: String,
    message: String,
}

#[cfg(test)]
mod tests {
    use crate::server::routers::fcm::client::FcmClient;
    use crate::server::routers::fcm::error::FcmError;
    use crate::server::routers::fcm::settings::FcmCredential;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;
    use url::Url;

    const PROJECT_ID: &str = "yup-test-243420";
    const ACCESS_TOKEN: &str = "ya29.c.ElouBywiys0LyNaZoLPJcp1Fdi2KjFMxzvYKLXkTdvM-rDfqKlvEq6PiMhGoGHx97t5FAvz3eb_ahdwlBjSStxHtDVQB4ZPRJQ_EOi-iS7PnayahU2S9Jp8S6rk";

    /// Write service data to a temporary file
    fn make_service_file() -> NamedTempFile {
        // Taken from the yup-oauth2 tests
        let contents = serde_json::json!({
            "type": "service_account",
            "project_id": PROJECT_ID,
            "private_key_id": "26de294916614a5ebdf7a065307ed3ea9941902b",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQDemmylrvp1KcOn\n9yTAVVKPpnpYznvBvcAU8Qjwr2fSKylpn7FQI54wCk5VJVom0jHpAmhxDmNiP8yv\nHaqsef+87Oc0n1yZ71/IbeRcHZc2OBB33/LCFqf272kThyJo3qspEqhuAw0e8neg\nLQb4jpm9PsqR8IjOoAtXQSu3j0zkXemMYFy93PWHjVpPEUX16NGfsWH7oxspBHOk\n9JPGJL8VJdbiAoDSDgF0y9RjJY5I52UeHNhMsAkTYs6mIG4kKXt2+T9tAyHw8aho\nwmuytQAfydTflTfTG8abRtliF3nil2taAc5VB07dP1b4dVYy/9r6M8Z0z4XM7aP+\nNdn2TKm3AgMBAAECggEAWi54nqTlXcr2M5l535uRb5Xz0f+Q/pv3ceR2iT+ekXQf\n+mUSShOr9e1u76rKu5iDVNE/a7H3DGopa7ZamzZvp2PYhSacttZV2RbAIZtxU6th\n7JajPAM+t9klGh6wj4jKEcE30B3XVnbHhPJI9TCcUyFZoscuPXt0LLy/z8Uz0v4B\nd5JARwyxDMb53VXwukQ8nNY2jP7WtUig6zwE5lWBPFMbi8GwGkeGZOruAK5sPPwY\nGBAlfofKANI7xKx9UXhRwisB4+/XI1L0Q6xJySv9P+IAhDUI6z6kxR+WkyT/YpG3\nX9gSZJc7qEaxTIuDjtep9GTaoEqiGntjaFBRKoe+VQKBgQDzM1+Ii+REQqrGlUJo\nx7KiVNAIY/zggu866VyziU6h5wjpsoW+2Npv6Dv7nWvsvFodrwe50Y3IzKtquIal\nVd8aa50E72JNImtK/o5Nx6xK0VySjHX6cyKENxHRDnBmNfbALRM+vbD9zMD0lz2q\nmns/RwRGq3/98EqxP+nHgHSr9QKBgQDqUYsFAAfvfT4I75Glc9svRv8IsaemOm07\nW1LCwPnj1MWOhsTxpNF23YmCBupZGZPSBFQobgmHVjQ3AIo6I2ioV6A+G2Xq/JCF\nmzfbvZfqtbbd+nVgF9Jr1Ic5T4thQhAvDHGUN77BpjEqZCQLAnUWJx9x7e2xvuBl\n1A6XDwH/ewKBgQDv4hVyNyIR3nxaYjFd7tQZYHTOQenVffEAd9wzTtVbxuo4sRlR\nNM7JIRXBSvaATQzKSLHjLHqgvJi8LITLIlds1QbNLl4U3UVddJbiy3f7WGTqPFfG\nkLhUF4mgXpCpkMLxrcRU14Bz5vnQiDmQRM4ajS7/kfwue00BZpxuZxst3QKBgQCI\nRI3FhaQXyc0m4zPfdYYVc4NjqfVmfXoC1/REYHey4I1XetbT9Nb/+ow6ew0UbgSC\nUZQjwwJ1m1NYXU8FyovVwsfk9ogJ5YGiwYb1msfbbnv/keVq0c/Ed9+AG9th30qM\nIf93hAfClITpMz2mzXIMRQpLdmQSR4A2l+E4RjkSOwKBgQCB78AyIdIHSkDAnCxz\nupJjhxEhtQ88uoADxRoEga7H/2OFmmPsqfytU4+TWIdal4K+nBCBWRvAX1cU47vH\nJOlSOZI0gRKe0O4bRBQc8GXJn/ubhYSxI02IgkdGrIKpOb5GG10m85ZvqsXw3bKn\nRVHMD0ObF5iORjZUqD0yRitAdg==\n-----END PRIVATE KEY-----\n",
            "client_email": "yup-test-sa-1@yup-test-243420.iam.gserviceaccount.com",
            "client_id": "102851967901799660408",
            "auth_uri": "https://accounts.google.com/o/oauth2/auth",
            "token_uri": mockito::server_url() + "/token",
            "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
            "client_x509_cert_url": "https://www.googleapis.com/robot/v1/metadata/x509/yup-test-sa-1%40yup-test-243420.iam.gserviceaccount.com"
        });

        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&serde_json::to_vec(&contents).unwrap())
            .unwrap();
        file
    }

    /// Mock the OAuth token endpoint to provide the access token
    fn mock_token_endpoint() -> mockito::Mock {
        mockito::mock("POST", "/token")
            .with_body(
                serde_json::json!({
                    "access_token": ACCESS_TOKEN,
                    "expires_in": 3600,
                    "token_type": "Bearer"
                })
                .to_string(),
            )
            .create()
    }

    /// Start building a mock for the FCM endpoint
    fn mock_fcm_endpoint_builder() -> mockito::Mock {
        mockito::mock(
            "POST",
            format!("/v1/projects/{}/messages:send", PROJECT_ID).as_str(),
        )
    }

    /// Make a FcmClient from the service auth data
    async fn make_client(auth_file: PathBuf) -> FcmClient {
        FcmClient::new(
            Url::parse(&mockito::server_url()).unwrap(),
            FcmCredential {
                auth_file,
                project_id: PROJECT_ID.to_string(),
            },
            reqwest::Client::new(),
        )
        .await
        .unwrap()
    }

    /// The FCM client uses the access token and parameters to build the
    /// expected FCM request.
    #[tokio::test]
    async fn sends_correct_request() {
        let service_file = make_service_file();
        let client = make_client(service_file.path().to_owned()).await;
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder()
            .match_header("Authorization", format!("Bearer {}", ACCESS_TOKEN).as_str())
            .match_header("Content-Type", "application/json; UTF-8")
            .match_body(r#"{"message":{"android":{"data":{"is_test":"true"},"ttl":"42s"},"token":"test-token"}}"#)
            .create();

        let mut data = HashMap::new();
        data.insert("is_test", "true".to_string());

        let result = client.send(data, "test-token".to_string(), 42).await;
        assert!(result.is_ok(), "result = {:?}", result);
        fcm_mock.assert();
    }

    /// Authorization errors are handled
    #[tokio::test]
    async fn unauthorized() {
        let service_file = make_service_file();
        let client = make_client(service_file.path().to_owned()).await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder()
            .with_status(401)
            .with_body(r#"{"error":{"status":"UNAUTHENTICATED","message":"test-message"}}"#)
            .create();

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), FcmError::FcmAuthentication),
            "result = {:?}",
            result
        );
    }

    /// 404 errors are handled
    #[tokio::test]
    async fn not_found() {
        let service_file = make_service_file();
        let client = make_client(service_file.path().to_owned()).await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder()
            .with_status(404)
            .with_body(r#"{"error":{"status":"NOT_FOUND","message":"test-message"}}"#)
            .create();

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), FcmError::FcmNotFound),
            "result = {:?}",
            result
        );
    }

    /// Unhandled errors (where an error object is returned) are wrapped and returned
    #[tokio::test]
    async fn other_fcm_error() {
        let service_file = make_service_file();
        let client = make_client(service_file.path().to_owned()).await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder()
            .with_status(400)
            .with_body(r#"{"error":{"status":"TEST_ERROR","message":"test-message"}}"#)
            .create();

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                FcmError::FcmUpstream { status, message }
                    if status == "TEST_ERROR" && message == "test-message"
            ),
            "result = {:?}",
            result
        );
    }

    /// Unknown errors (where an error object is NOT returned) is handled
    #[tokio::test]
    async fn unknown_fcm_error() {
        let service_file = make_service_file();
        let client = make_client(service_file.path().to_owned()).await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder()
            .with_status(400)
            .with_body("{}")
            .create();

        let result = client
            .send(HashMap::new(), "test-token".to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), FcmError::FcmUnknown),
            "result = {:?}",
            result
        );
    }
}
