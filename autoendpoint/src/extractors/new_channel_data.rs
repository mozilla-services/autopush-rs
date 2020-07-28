use uuid::Uuid;

/// The data provided when creating a new channel for an existing user. Extract
/// from the request via the `Json` extractor.
#[derive(serde::Deserialize, Default)]
pub struct NewChannelData {
    #[serde(rename = "channelID")]
    pub channel_id: Option<Uuid>,
    pub key: Option<String>,
}
