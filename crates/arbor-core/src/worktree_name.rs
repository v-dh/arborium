pub fn sanitize_worktree_name(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;

    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character.to_ascii_lowercase());
            previous_dash = false;
            continue;
        }

        if character == '-' || character == '_' {
            sanitized.push(character);
            previous_dash = false;
            continue;
        }

        if !previous_dash && !sanitized.is_empty() {
            sanitized.push('-');
            previous_dash = true;
        }
    }

    while sanitized.ends_with('-') {
        let _ = sanitized.pop();
    }

    sanitized
}

#[cfg(test)]
mod tests {
    use super::sanitize_worktree_name;

    #[test]
    fn sanitize_worktree_name_normalizes_symbols() {
        assert_eq!(
            sanitize_worktree_name("  Fix auth / callback race!  "),
            "fix-auth-callback-race"
        );
        assert_eq!(sanitize_worktree_name("ARB-42_bugfix"), "arb-42_bugfix");
    }

    #[test]
    fn sanitize_worktree_name_removes_invalid_dot_sequences() {
        assert_eq!(
            sanitize_worktree_name("Issue 123 Fix parser... now."),
            "issue-123-fix-parser-now"
        );
        assert_eq!(sanitize_worktree_name("release.v1.2"), "release-v1-2");
        assert_eq!(sanitize_worktree_name("ends-with-dot."), "ends-with-dot");
    }
}
