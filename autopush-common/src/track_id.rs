use std::{fmt, str::FromStr};

use uuid;

use crate::errors::ApcError;

pub const SEPARATOR: &str = "#";

/// A tracking ID for Mozilla generated and consumed messages.
/// This ID is for Autopush diagnostic purposes and requires a message to be
/// flagged for tracking by matching against the Public VAPID key.
///
/// This will generally be called inside of Autoendpoint, during the extraction
/// phase as something like:
///
/// ```rust, ignore
/// //
/// let track_id = TrackId{meta: self.get_meta(), ..Default::default()};
/// ```
///
#[derive(Clone, Debug)]
pub struct TrackId {
    /// An opaque identifier. This should be universally unique and will be a
    /// Bigtable key.
    pub id: String,
    /// An optional, externally provided specifier.
    pub meta: Option<String>,
}

impl Default for TrackId {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            meta: None,
        }
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.id,
            self.meta
                .clone()
                .map(|v| format!("{SEPARATOR}{v}"))
                .unwrap_or_default()
        )
    }
}

/// TrackIds generally will be String values that are stored
/// and retrieved. They can persist as String values, but it may
/// be useful to revert to the struct.
impl FromStr for TrackId {
    type Err = ApcError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(SEPARATOR).collect();
        let meta = if parts.len() == 1 {
            None
        } else {
            Some(parts[1].to_owned())
        };
        Ok(Self {
            id: parts[0].to_owned(),
            meta,
        })
    }
}
