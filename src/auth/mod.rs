use std::net::IpAddr;

use crate::config::ProfileConfig;

pub fn resolve_profile<'a>(
    profiles: &'a [ProfileConfig],
    token: &str,
) -> Option<&'a ProfileConfig> {
    profiles
        .iter()
        .find(|profile| profile.api_keys.iter().any(|key| key == token))
}

// An empty allowlist means no IP restriction is configured (allow all).
pub fn validate_ip(allowed_ips: &[String], ip: &IpAddr) -> bool {
    allowed_ips.is_empty() || allowed_ips.iter().any(|allowed| allowed == &ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoutingConfig;

    fn profile(name: &str, keys: &[&str]) -> ProfileConfig {
        ProfileConfig {
            name: name.to_string(),
            api_keys: keys.iter().map(|k| k.to_string()).collect(),
            allowed_ips: vec![],
            routing: RoutingConfig::default(),
            rate_limit: None,
            cost_limit: None,
        }
    }

    #[test]
    fn resolves_known_key_to_its_profile() {
        let profiles = vec![profile("desktop", &["gw_abc"])];
        let resolved = resolve_profile(&profiles, "gw_abc").unwrap();
        assert_eq!(resolved.name, "desktop");
    }

    #[test]
    fn returns_none_for_unknown_key() {
        let profiles = vec![profile("desktop", &["gw_abc"])];
        assert!(resolve_profile(&profiles, "gw_wrong").is_none());
    }

    #[test]
    fn returns_none_for_empty_token() {
        let profiles = vec![profile("desktop", &["gw_abc"])];
        assert!(resolve_profile(&profiles, "").is_none());
    }

    #[test]
    fn distinguishes_between_multiple_profiles() {
        let profiles = vec![
            profile("desktop", &["gw_desktop"]),
            profile("partner-a", &["gw_partner"]),
        ];

        assert_eq!(
            resolve_profile(&profiles, "gw_desktop").unwrap().name,
            "desktop"
        );
        assert_eq!(
            resolve_profile(&profiles, "gw_partner").unwrap().name,
            "partner-a"
        );
    }

    #[test]
    fn empty_allowlist_allows_any_ip() {
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(validate_ip(&[], &ip));
    }

    #[test]
    fn accepts_listed_ip() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(validate_ip(&["127.0.0.1".to_string()], &ip));
    }

    #[test]
    fn rejects_unlisted_ip() {
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(!validate_ip(&["127.0.0.1".to_string()], &ip));
    }
}
