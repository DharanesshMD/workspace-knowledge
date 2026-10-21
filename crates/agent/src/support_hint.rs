//! Plain-language guidance for the machine-readable codes that unsupported client
//! versions and platforms produce. The code itself is never changed.
const TABLE: &str = "see docs/adapters/support.md";
/// Guidance for a refusal code, or `None` when the code is not a support boundary.
pub fn explain(code: &str) -> Option<String> {
    let text = match code {
        "cursor_version_unverified" => "the installed Cursor is not the version this capture adapter was tested with, so private recovery stays off rather than guessing at an unverified transcript format",
        "cursor_version_unavailable" => "Cursor's installed version could not be read from /Applications/Cursor.app, so private recovery stays off",
        "native_enrollment_changed" => "the enrolled client version, workspace or consent no longer matches what was approved (a client update changes the version); re-enroll after checking that the new version is supported",
        "native_platform_unsupported" => "native capture is only supported on macOS; use the assistant-written flow or an explicit export instead",
        _ => return None,
    };
    Some(format!("{text}; {TABLE}"))
}
#[cfg(test)]
mod tests {
    use super::explain;
    #[test]
    fn every_support_boundary_code_explains_itself_and_points_at_the_table() {
        for code in [
            "cursor_version_unverified",
            "cursor_version_unavailable",
            "native_enrollment_changed",
            "native_platform_unsupported",
        ] {
            let text = explain(code).unwrap();
            assert!(
                text.contains("docs/adapters/support.md") && text.len() > 60,
                "{code}"
            );
        }
        assert!(explain("home_unavailable").is_none());
    }
}
