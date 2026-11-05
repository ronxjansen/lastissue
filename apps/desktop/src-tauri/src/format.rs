//! Display helpers for names (dates are formatted in the front-end).

/// `Ada Lovelace <ada@example.com>` → `Ada Lovelace`; a bare address →
/// its local part.
pub fn display_name(addr: &str) -> String {
    let addr = addr.trim();
    if let Some((name, _)) = addr.split_once('<') {
        let name = name.trim().trim_matches('"').trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    let email = addr.trim_start_matches('<').trim_end_matches('>');
    email.split('@').next().unwrap_or(email).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            display_name("Ada Lovelace <ada@example.com>"),
            "Ada Lovelace"
        );
        assert_eq!(display_name("\"Bob\" <bob@example.com>"), "Bob");
        assert_eq!(display_name("<carol@example.com>"), "carol");
        assert_eq!(display_name("dave@example.com"), "dave");
    }
}
