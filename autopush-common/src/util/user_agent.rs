use woothee::parser::Parser;

// List of valid user-agent attributes to keep, anything not in this
// list is considered 'Other'. We log the user-agent on connect always
// to retain the full string, but for DD more tags are expensive so we
// limit to these.
const VALID_UA_BROWSER: &[&str] = &["Chrome", "Firefox", "Safari", "Opera"];

// See dataset.rs in https://github.com/woothee/woothee-rust for the
// full list (WootheeResult's 'os' field may fall back to its 'name'
// field). Windows has many values and we only care that its Windows
//
// The mobile entries matter for the bridge registration API, which only
// mobile clients call: without them every iOS and Android check-in collapses
// into `Other`, hiding the platform split that separates APNS from FCM.
// iOS reports the device rather than the OS, so it arrives as three values.
const VALID_UA_OS: &[&str] = &[
    "Firefox OS",
    "Linux",
    "Mac OSX",
    "Android",
    "iPhone",
    "iPad",
    "iPod",
];

/// The tag value for a user agent we could not resolve.
const UA_METRIC_UNKNOWN: &str = "Other";

#[derive(Clone, Debug)]
pub struct UserAgentInfo {
    _user_agent_string: String,
    pub category: String,
    pub browser_name: String,
    pub browser_version: String,
    pub metrics_browser: String,
    pub metrics_os: String,
    pub os_version: String,
    pub os: String,
    // Note, Woothee can determine if a user agent is mobile if the
    // ["smartphone", "mobilephone"].contains(category)
}

impl Default for UserAgentInfo {
    /// Used when a request carries no `User-Agent` header at all.
    ///
    /// The raw fields stay empty so logs can tell a missing header from an
    /// unparsable one, but the `metrics_*` fields take the same
    /// [UA_METRIC_UNKNOWN] bucket an unparsable header gets, to avoid
    /// sending empty tags to metrics backend.
    fn default() -> Self {
        Self {
            _user_agent_string: String::new(),
            category: String::new(),
            browser_name: String::new(),
            browser_version: String::new(),
            metrics_browser: UA_METRIC_UNKNOWN.to_owned(),
            metrics_os: UA_METRIC_UNKNOWN.to_owned(),
            os_version: String::new(),
            os: String::new(),
        }
    }
}

impl From<&str> for UserAgentInfo {
    fn from(user_agent_string: &str) -> Self {
        let parser = Parser::new();
        let wresult = parser.parse(user_agent_string).unwrap_or_default();

        // Determine a base os/browser for metrics' tags
        let metrics_os = if wresult.os.starts_with("Windows") {
            "Windows"
        } else if VALID_UA_OS.contains(&wresult.os) {
            wresult.os
        } else {
            UA_METRIC_UNKNOWN
        };
        let metrics_browser = if VALID_UA_BROWSER.contains(&wresult.name) {
            wresult.name
        } else {
            UA_METRIC_UNKNOWN
        };

        Self {
            category: wresult.category.to_owned(),
            browser_name: wresult.name.to_owned(),
            browser_version: wresult.version.to_owned(),
            metrics_browser: metrics_browser.to_owned(),
            metrics_os: metrics_os.to_owned(),
            os_version: wresult.os_version.to_string(),
            os: wresult.os.to_owned(),
            _user_agent_string: user_agent_string.to_owned(),
        }
    }
}

impl From<&actix_web::HttpRequest> for UserAgentInfo {
    fn from(req: &actix_web::HttpRequest) -> UserAgentInfo {
        if let Some(header) = req.headers().get(&actix_web::http::header::USER_AGENT) {
            Self::from(header.to_str().unwrap_or("UNKNOWN"))
        } else {
            UserAgentInfo::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UserAgentInfo;

    #[test]
    fn test_linux() {
        let agent = r#"Mozilla/5.0 (X11; U; Linux i686; en-US; rv:1.9.1.2) Gecko/20090807 Mandriva Linux/1.9.1.2-1.1mud2009.1 (2009.1) Firefox/3.5.2 FirePHP/0.3,gzip(gfe),gzip(gfe)"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "Linux");
        assert_eq!(ua_result.os, "Linux");
        assert_eq!(ua_result.metrics_browser, "Firefox");
    }

    #[test]
    fn test_windows() {
        let agent = r#"Mozilla/5.0 (Windows; U; Windows NT 6.1; en-US; rv:1.9.2.3) Gecko/20100401 Firefox/3.6.3 (.NET CLR 3.5.30729)"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "Windows");
        assert_eq!(ua_result.os, "Windows 7");
        assert_eq!(ua_result.metrics_browser, "Firefox");
    }

    #[test]
    fn test_osx() {
        let agent =
            r#"Mozilla/5.0 (Macintosh; Intel Mac OS X 10.5; rv:2.1.1) Gecko/ Firefox/5.0.1"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "Mac OSX");
        assert_eq!(ua_result.os, "Mac OSX");
        assert_eq!(ua_result.metrics_browser, "Firefox");
    }

    /// Firefox iOS reports the device, not the OS, so each iOS device kind is
    /// its own `metrics_os` value.
    #[test]
    fn test_firefox_ios() {
        let agent = r#"Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/127.0 Mobile/15E148 Safari/605.1.15"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "iPhone");
        assert_eq!(ua_result.os, "iPhone");
        // woothee resolves `FxiOS/` to Firefox despite the WebKit shell, so
        // the browser can't distinguish iOS from Android. `os` is what does.
        assert_eq!(ua_result.metrics_browser, "Firefox");

        let ipad = r#"Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/127.0 Mobile/15E148 Safari/605.1.15"#;
        assert_eq!(UserAgentInfo::from(ipad).metrics_os, "iPad");
    }

    #[test]
    fn test_firefox_android() {
        let agent = r#"Mozilla/5.0 (Android 14; Mobile; rv:127.0) Gecko/127.0 Firefox/127.0"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "Android");
        assert_eq!(ua_result.os, "Android");
        assert_eq!(ua_result.metrics_browser, "Firefox");
    }

    /// A request with no `User-Agent` header must still produce a usable tag
    /// value (default).
    #[test]
    fn test_missing_user_agent() {
        let ua_result = UserAgentInfo::default();
        assert_eq!(ua_result.metrics_os, "Other");
        assert_eq!(ua_result.metrics_browser, "Other");
        // the raw fields stay empty, so logs can still tell a missing header
        // apart from an unparsable one (which yields woothee's "UNKNOWN")
        assert_eq!(ua_result.os, "");
        assert_eq!(ua_result.browser_name, "");
    }

    #[test]
    fn test_other() {
        let agent =
            r#"BlackBerry9000/4.6.0.167 Profile/MIDP-2.0 Configuration/CLDC-1.1 VendorID/102"#;
        let ua_result = UserAgentInfo::from(agent);
        assert_eq!(ua_result.metrics_os, "Other");
        assert_eq!(ua_result.os, "BlackBerry");
        assert_eq!(ua_result.metrics_browser, "Other");
        assert_eq!(ua_result.browser_name, "UNKNOWN");
    }
}
