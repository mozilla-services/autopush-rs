use std::collections::HashMap;
use std::time;

use super::error::WnsError;

use super::settings::WnsChannelSettings;

/// Access token and info for WNS. These are per
/// profile.
#[allow(unused)]
#[derive(Debug, serde::Deserialize, Clone)]
pub struct WnsAccessToken {
    token_type: String,
    pub expires_in: time::Duration,
    ext_expires_in: time::Duration,
    pub expires_on: u64,
    not_before: u64,
    pub access_token: String,
}

#[derive(Debug, Clone)]
pub struct WnsClient {
    profile: WnsChannelSettings,
    client: reqwest::Client,
}

impl WnsClient {
    pub fn new(profile: &WnsChannelSettings, client: reqwest::Client) -> Result<Self, WnsError> {
        Ok(Self {
            profile: profile.clone(),
            client,
        })
    }

    /// Fetch an access token from the WNS service.
    ///
    /// Note that this is a mutable. Ideally, this token would be stored in the router, however,
    /// the routers were designed to be immutable.
    pub async fn get_access_token(&self, app_id: &str) -> Result<WnsAccessToken, WnsError> {
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", app_id),
            ("client_secret", &self.profile.secret),
            ("scope", "https://wns.windows.com/.default"),
        ];

        let response = self
            .client
            .post(format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                &self.profile.tenant_id
            ))
            .form(&params)
            .send()
            .await
            .map_err(|e| WnsError::General(format!("Failed to fetch access token: {}", e)))?;
        if !response.status().is_success() {
            return Err(WnsError::General(format!(
                "Failed to fetch access token: HTTP {}",
                response.status()
            )));
        }
        let token_response = response.json::<WnsAccessToken>().await.map_err(|err| {
            WnsError::General(format!("Failed to parse access token response: {}", err))
        })?;
        Ok(token_response)
    }

    pub async fn send(
        &self,
        notification: HashMap<&str, String>,
        channel_token: &str,
        access_token: &str,
        ttl: u64,
    ) -> Result<(), WnsError> {
        //TODO implement actual sending logic
        // Build the transmitted notification
        // {
        //     "meta": {},
        //     "data": notification
        // }
        let meta: HashMap<&str, String> = HashMap::new();
        let message: HashMap<&str, HashMap<&str, String>> =
            HashMap::from([("meta", meta), ("data", notification)]);
        let body = serde_json::to_string(&message)
            .map_err(|e| WnsError::General(format!("Could not encode message: {}", e)))?;
        self.client
            .post(format!("!?token={}", channel_token))
            .header("Content-Type", "application/octet-stream")
            .header("X-WNS-Type", "wns/raw")
            .header("X-WNS-TTL", ttl.to_string())
            .header("Authorization", format!("Bearer {}", access_token))
            .body(body)
            .send()
            .await
            .map_err(|e| WnsError::General(format!("Failed to send notification: {}", e)))?;
        Ok(())
    }
}
