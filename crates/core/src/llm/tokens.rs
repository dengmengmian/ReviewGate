//! 轻量 token 估算。
//!
//! 没有引入真正的 tokenizer（各 provider 分词器不同，且对"是否会撞上下文窗口"这种
//! **闸门判断**来说精确分词是过度工程）。这里用保守启发式：**偏高估**，宁可早切/早降级，
//! 也不要漏判溢出。用途仅限发送前的预算守卫，不用于计费（计费看 API 回传的真实 `Usage`）。

/// 估算一段文本的 token 数（保守偏高）。
///
/// 经验上英文代码约 3–4 char/token、CJK 约 1–2 char/token。取 `chars/3` 向上取整，
/// 对纯英文略高估（安全），对中文注释也不会过低估。空串记 0。
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn ascii_order_of_magnitude() {
        // 30 个 ASCII 字符 → ~10 token。
        assert_eq!(estimate_tokens(&"a".repeat(30)), 10);
        // 单字符不被抹成 0（div_ceil）。
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn cjk_counts_by_chars_not_bytes() {
        // 中文每字 3 字节，但按 char 计：9 字 → 3 token，而非按字节 27/3=9。
        let s = "审查门禁很重要啊哈"; // 9 个汉字
        assert_eq!(s.chars().count(), 9);
        assert_eq!(estimate_tokens(s), 3);
    }

    #[test]
    fn monotonic_with_length() {
        assert!(estimate_tokens(&"x".repeat(1000)) > estimate_tokens(&"x".repeat(100)));
    }
}
