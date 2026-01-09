pub(crate) fn parse_bool_loose(s: &str) -> bool {
    let v = s.trim().to_ascii_lowercase();
    matches!(v.as_str(), "1" | "true" | "on" | "yes")
}
