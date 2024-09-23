/// Push Reliability Recorder
use crate::errors::{ApcError, ApcErrorKind};

/// The various states that a message may transit on the way from reception to delivery.
#[derive(Debug)]
pub enum PushReliabilityStates {
    RECEIVED,
    STORED,
    RETRIEVED,
    TRANSMITTED,
    ACCEPTED,
    DELIVERED,
}

// TODO: Differentiate between "transmitted via webpush" and "transmitted via bridge"?
impl std::fmt::Display for PushReliabilityStates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RECEIVED => "received",
            Self::STORED => "stored",
            Self::RETRIEVED => "retrieved",
            Self::TRANSMITTED => "transmitted",
            Self::ACCEPTED => "accepted",
            Self::DELIVERED => "delivered",
        })
    }
}

impl std::str::FromStr for PushReliabilityStates {
    type Err = ApcError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "recieved" => Self::RECEIVED,
            "stored" => Self::STORED,
            "retrieved" => Self::RETRIEVED,
            "transmitted" => Self::TRANSMITTED,
            "accepted" => Self::ACCEPTED,
            "delivered" => Self::DELIVERED,
            _ => {
                return Err(ApcErrorKind::GeneralError("Unknown tracker state".to_owned()).into());
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct PushReliability {
    record_host: String,
    record_port: u32,
}

impl PushReliability {
    // Do the magic to make a report instance, whatever that will be.
    pub fn new(host: String, port: u32) -> Self {
        Self {
            record_host: host,
            record_port: port,
        }
    }

    // Handle errors internally.
    pub async fn record(&self, reliability_id: &Option<String>, state: PushReliabilityStates) {
        if reliability_id.is_none() {
            return;
        }
        // TODO: Record this to the reporting system.
        // NO-OP things for now.
        let _ = self.record_host;
        let _ = self.record_port;
        let _ = reliability_id;
        let _ = state;
    }
}
