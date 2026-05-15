use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct OutputShape {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    pub wall_time_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_token_count: Option<usize>,
    pub output: String,
}

pub fn approx_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

pub fn truncate_head_tail(s: &str, max_tokens: usize) -> (String, Option<usize>) {
    let original = approx_tokens(s);
    if original <= max_tokens {
        return (s.to_string(), Some(original));
    }
    let max_chars = max_tokens.saturating_mul(4);
    let marker =
        format!("\n\n[agentbox output truncated: original approximately {original} tokens]\n\n");
    if max_chars <= marker.len() + 16 {
        return (marker, Some(original));
    }
    let keep = max_chars - marker.len();
    let head_chars = keep / 2;
    let tail_chars = keep - head_chars;
    let head: String = s.chars().take(head_chars).collect();
    let tail: String = s
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (format!("{head}{marker}{tail}"), Some(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_head_tail() {
        let s = "a".repeat(1000);
        let (out, orig) = truncate_head_tail(&s, 20);
        assert!(out.contains("agentbox output truncated"));
        assert_eq!(orig, Some(250));
    }

    #[test]
    fn invalid_utf8_is_lossy_before_truncation() {
        let s = String::from_utf8_lossy(&[0xff, b'a']).to_string();
        assert!(s.contains('�'));
    }
}
