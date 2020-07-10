use crate::headers::util::split_key_value;
use std::collections::HashMap;

/// Parses the Crypto-Key header (and similar headers) described by
/// http://tools.ietf.org/html/draft-ietf-httpbis-encryption-encoding-00#section-4
pub struct CryptoKeyHeader {
    /// The sections (comma separated) and their items (key-value semicolon separated)
    sections: Vec<HashMap<String, String>>,
}

impl CryptoKeyHeader {
    /// Parse a Crypto-Key header
    pub fn parse(header: &str) -> Option<Self> {
        let mut sections = Vec::new();

        for section_str in header.split(',') {
            let mut section = HashMap::new();

            for item_str in section_str.split(';') {
                let (key, value) = split_key_value(item_str)?;

                section.insert(
                    key.trim().to_owned(),
                    value.trim_matches(&[' ', '"'] as &[char]).to_owned(),
                );
            }

            sections.push(section);
        }

        Some(Self { sections })
    }

    /// Get the value of the first item with the given key
    pub fn get_by_key(&self, key: &str) -> Option<&str> {
        for section in &self.sections {
            if let Some(value) = section.get(key) {
                return Some(value.as_str());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::CryptoKeyHeader;

    const TEST_HEADER: &str = "keyid=\"p256dh\";dh=\"BDw9T0eImd4ax818VcYqDK_DOhcuDswKero\
        YyNkdhYmygoLSDlSiWpuoWYUSSFxi25cyyNTR5k9Ny93DzZc0UI4\",\
        p256ecdsa=\"BF92zdI_AKcH5Q31_Rr-04bPqOHU_Qg6lAawHbvfQrY\
        xV_vIsAsHSyaiuyfofvxT8ZVIXccykd4V2Z7iJVfreT8\"";

    #[test]
    fn parse_succeeds() {
        assert!(CryptoKeyHeader::parse(TEST_HEADER).is_some())
    }

    /// All items are parsed correctly
    #[test]
    fn parse_all_items() {
        let crypto_keys = CryptoKeyHeader::parse(TEST_HEADER).unwrap();

        assert_eq!(crypto_keys.get_by_key("keyid"), Some("p256dh"));
        assert_eq!(
            crypto_keys.get_by_key("dh"),
            Some(
                "BDw9T0eImd4ax818VcYqDK_DOhcuDswKeroYyNkdhYm\
                 ygoLSDlSiWpuoWYUSSFxi25cyyNTR5k9Ny93DzZc0UI4"
            )
        );
        assert_eq!(
            crypto_keys.get_by_key("p256ecdsa"),
            Some(
                "BF92zdI_AKcH5Q31_Rr-04bPqOHU_Qg6lAawHbvfQrY\
                 xV_vIsAsHSyaiuyfofvxT8ZVIXccykd4V2Z7iJVfreT8"
            )
        )
    }

    /// Accessing an unknown item returns None
    #[test]
    fn get_unknown() {
        let crypto_keys = CryptoKeyHeader::parse(TEST_HEADER).unwrap();

        assert!(crypto_keys.get_by_key("unknown").is_none());
    }

    /// Parsing an invalid header (no equals sign in item) returns None
    #[test]
    fn parse_invalid() {
        assert!(CryptoKeyHeader::parse("key=value;invalid").is_none());
    }
}
