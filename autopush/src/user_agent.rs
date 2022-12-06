use woothee::parser::Parser;

// List of valid user-agent attributes to keep, anything not in this
// list is considered 'Other'. We log the user-agent on connect always
// to retain the full string, but for DD more tags are expensive so we
// limit to these.
const VALID_UA_BROWSER: &[&str] = &["Chrome", "Firefox", "Safari", "Opera"];

// See dataset.rs in https://github.com/woothee/woothee-rust for the
// full list (WootheeResult's 'os' field may fall back to its 'name'
// field). Windows has many values and we only care that its Windows
const VALID_UA_OS: &[&str] = &["Firefox OS", "Linux", "Mac OSX"];

#[derive(Clone, Debug, Default)]
pub struct UserAgentInfo {
    _user_agent_string: String,
    pub category: String,
    pub browser_name: String,
    pub browser_version: String,
    pub metrics_browser: String,
    pub metrics_os: String,
    pub os_version: String,
    pub os: String,
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
            "Other"
        };
        let metrics_browser = if VALID_UA_BROWSER.contains(&wresult.name) {
            wresult.name
        } else {
            "Other"
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

/*
impl UserAgentInfo {
    pub fn parsed(&self) -> WootheeResult {
        let parser = Parser::new();
        parser.parse(&self._user_agent_string).unwrap_or_default()
    }
}
*/

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
