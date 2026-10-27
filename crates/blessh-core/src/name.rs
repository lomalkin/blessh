//! Server names: what a server advertises and a client selects it by.
//!
//! They follow host name rules (RFC 1123 labels): letters, digits and
//! hyphens, labels separated by dots, no label starting or ending with a
//! hyphen, compared without regard to ASCII case. A name must also fit the
//! scan response of a legacy advertisement.

/// Longest name in bytes: a legacy scan response holds 31 bytes, two of
/// which are the AD structure's length and type.
pub const MAX_LEN: usize = 29;

/// Why `name` is not a valid server name, if it is not.
pub fn check(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name.len() > MAX_LEN {
        return Err(format!("the name is longer than {MAX_LEN} bytes"));
    }
    for label in name.split('.') {
        if label.is_empty() {
            return Err("the name has an empty label (two dots, or a dot at an end)".into());
        }
        if !label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("the name may contain only letters, digits, hyphens and dots".into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("a part of the name starts or ends with a hyphen".into());
        }
    }
    Ok(())
}

/// Whether two names are the same name (ASCII case-insensitive, like host
/// names; a trailing dot is ignored).
pub fn same(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.strip_suffix('.').unwrap_or(s).to_owned();
    trim(a).eq_ignore_ascii_case(&trim(b))
}

/// A valid name derived from a host or computer name, for a default: the
/// first label, without a conflict suffix macOS adds (" (2)"), other
/// characters replaced by hyphens, cut to `MAX_LEN`.
pub fn derive(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    // macOS renames a computer "name (2)" on a conflict.
    if let Some(open) = s.rfind(" (") {
        let inner = &s[open + 2..];
        if inner.ends_with(')') && inner[..inner.len() - 1].bytes().all(|c| c.is_ascii_digit()) {
            s = &s[..open];
        }
    }
    let first = s.split('.').next().unwrap_or("");
    let mut out = String::new();
    for c in first.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out = out.trim_matches('-').to_owned();
    out.truncate(MAX_LEN);
    let out = out.trim_end_matches('-').to_owned();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_names() {
        assert!(check("my-pi").is_ok());
        assert!(check("Pi3.lab").is_ok());
        assert!(check("x").is_ok());
        assert!(check("").is_err());
        assert!(check("-pi").is_err());
        assert!(check("pi-").is_err());
        assert!(check("pi..lab").is_err());
        assert!(check("my pi").is_err());
        assert!(check("pi_3").is_err());
        assert!(check(&"a".repeat(30)).is_err());
        assert!(check(&"a".repeat(29)).is_ok());
    }

    #[test]
    fn compares_like_host_names() {
        assert!(same("My-Pi", "my-pi"));
        assert!(same("pi.", "PI"));
        assert!(!same("pi", "pi2"));
    }

    #[test]
    fn derives_defaults() {
        assert_eq!(derive("laptop (6303)").as_deref(), Some("laptop"));
        assert_eq!(derive("Mac.mynet").as_deref(), Some("Mac"));
        assert_eq!(derive("box-1\n").as_deref(), Some("box-1"));
        assert_eq!(
            derive("Alice's MacBook Pro").as_deref(),
            Some("Alice-s-MacBook-Pro")
        );
        assert_eq!(derive("lab (pi)").as_deref(), Some("lab-pi"));
        assert_eq!(derive("  ").as_deref(), None);
        assert_eq!(derive("Ünïcode").as_deref(), Some("n-code"));
        assert_eq!(derive(&"b".repeat(40)).map(|s| s.len()), Some(29));
    }
}
