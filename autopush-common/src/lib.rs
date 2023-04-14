#![recursion_limit = "1024"]

#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_scope;

#[macro_use]
pub mod db;
pub mod endpoint;
pub mod errors;
pub mod logging;
pub mod metrics;
pub mod notification;
// pending actix 4:
pub mod tags;

#[macro_use]
pub mod util;

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct Uaid(Uuid);

impl fmt::Display for Uaid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_simple())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::Uaid;

    pub const DUMMY_UAID: Uuid = Uuid::from_u128(0xdeadbeef_0000_0000_deca_fbad00000000);
    pub const DUMMY_UAID_JSON: &str = r#""deadbeef-0000-0000-deca-fbad00000000""#;

    #[test]
    fn uaid_ser_de() {
        // Renders as hyphenated (Uuid::to_string() is also hyphenated)
        let uaid1 = Uaid(DUMMY_UAID);
        assert_eq!(serde_json::to_string(&uaid1).unwrap(), DUMMY_UAID_JSON);

        let uaid2: Uaid = serde_json::from_str(&DUMMY_UAID_JSON).unwrap();
        assert_eq!(uaid1, uaid2);
    }

    #[test]
    fn uaid_to_string() {
        // However to_string is simple
        let uaid = Uaid(DUMMY_UAID);
        assert_eq!(uaid.to_string(), DUMMY_UAID.as_simple().to_string());
    }
}
