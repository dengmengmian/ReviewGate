//! 轻量 token 估算。
//!
//! 没有引入真正的 tokenizer（各 provider 分词器不同，且对"是否会撞上下文窗口"这种
//! **闸门判断**来说精确分词是过度工程）。这里用保守启发式：**偏高估**，宁可早切/早降级，
//! 也不要漏判溢出。用途仅限发送前的预算守卫，不用于计费（计费看 API 回传的真实 `Usage`）。

/// 估算一段文本的 token 数（保守偏高），按 ASCII / 非 ASCII 分桶以贴近各家分词器的真实粒度。
///
/// - ASCII（代码/英文）：约 3–4 char/token，取 `/3` 向上取整 → 对代码略高估（安全）。
/// - 非 ASCII（CJK 等）：现代中文模型约 1.5–2.5 char/token（≈0.5 token/char），按 `2/3 token/char`
///   计 → 略偏保守。**注意**：旧实现对全体统一 `/3`，对 CJK 是**低估**（不安全方向），本版修正之。
///
/// 用途仅限发送前预算守卫，不用于计费（计费看 API 回传的真实 `Usage`）。空串记 0。
pub fn estimate_tokens(s: &str) -> usize {
    let (mut ascii, mut wide) = (0usize, 0usize);
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    ascii.div_ceil(3) + (wide * 2).div_ceil(3)
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
        // 中文每字 3 字节，但按 char 计（不按字节）。9 个非 ASCII 字 → ceil(9*2/3)=6 token
        // （比旧版统一 /3 的 3 token 更贴近真实——旧版对 CJK 是低估，属不安全方向）。
        let s = "审查门禁很重要啊哈"; // 9 个汉字
        assert_eq!(s.chars().count(), 9);
        assert_eq!(estimate_tokens(s), 6);
    }

    #[test]
    fn mixed_ascii_and_cjk_buckets_add_up() {
        // 6 个 ASCII（→ 2）+ 3 个 CJK（→ ceil(6/3)=2）= 4。
        let s = "abcdef审查门";
        assert_eq!(estimate_tokens(s), 2 + 2);
    }

    #[test]
    fn monotonic_with_length() {
        assert!(estimate_tokens(&"x".repeat(1000)) > estimate_tokens(&"x".repeat(100)));
    }
}
