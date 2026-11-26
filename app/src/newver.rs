//! Whether a newer Whilo Trace is out: once a day, and only when the user said yes to it. One GET
//! of GitHub's latest release; the version is compared with this program's. Nothing is
//! downloaded: the interface offers the release page.

/// The latest published release (drafts and prereleases are not "latest").
pub const LATEST_API: &str = "https://api.github.com/repos/Whilo-Labs/whilo-trace/releases/";
/// What the interface opens.
pub const RELEASES_PAGE: &str = "https://github.com/Whilo-Labs/whilo-trace/releases/latest";

/// "1.2.3" or "v1.2.3" as numbers; anything after a `-` is ignored.
fn parts(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

/// `a` is a later version than `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    match (parts(a), parts(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The version of the latest release in GitHub's answer when it is newer than `current`.
pub fn newer_in(latest_json: &str, current: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(latest_json).ok()?;
    let tag = v.get("tag_name")?.as_str()?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    is_newer(&version, current).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("v1.0.0", "0.4.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(!is_newer("junk", "0.4.0"));
    }

    #[test]
    fn the_latest_release_is_read_from_its_tag() {
        let json = r#"{"tag_name": "v0.5.0", "name": "Whilo Trace 0.5.0"}"#;
        assert_eq!(newer_in(json, "0.4.0").as_deref(), Some("0.5.0"));
        assert_eq!(newer_in(json, "0.5.0"), None);
        assert_eq!(newer_in("{}", "0.4.0"), None);
    }
}
