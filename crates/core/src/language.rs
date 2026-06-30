//! Output language detection for LLM-facing prompts.

/// Detect the user's preferred output language.
///
/// Precedence:
/// 1. `REVIEWGATE_OUTPUT_LANGUAGE` for explicit control.
/// 2. Locale environment (`LC_ALL`, `LC_MESSAGES`, `LANG`).
/// 3. English fallback for portable CI defaults.
pub fn output_language() -> String {
    detect_output_language_from(std::env::vars())
}

pub(crate) fn detect_output_language_from<I, K, V>(vars: I) -> String
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let vars: Vec<(String, String)> = vars
        .into_iter()
        .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
        .collect();

    if let Some(explicit) = vars
        .iter()
        .find(|(k, _)| k == "REVIEWGATE_OUTPUT_LANGUAGE")
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
    {
        return explicit.to_string();
    }

    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(locale) = vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
        {
            return language_from_locale(locale);
        }
    }

    "English".into()
}

fn language_from_locale(locale: &str) -> String {
    let l = locale.to_ascii_lowercase();
    let tag = l
        .split(['.', '@'])
        .next()
        .unwrap_or(l.as_str())
        .replace('-', "_");
    if tag == "c" || tag == "posix" {
        return "English".into();
    }
    if tag.starts_with("zh_tw") || tag.starts_with("zh_hk") || tag.starts_with("zh_mo") {
        return "Chinese (Traditional)".into();
    }
    if tag.starts_with("zh") {
        return "Chinese (Simplified)".into();
    }
    if tag.starts_with("ja") {
        return "Japanese".into();
    }
    if tag.starts_with("ko") {
        return "Korean".into();
    }
    if tag.starts_with("fr") {
        return "French".into();
    }
    if tag.starts_with("de") {
        return "German".into();
    }
    if tag.starts_with("es") {
        return "Spanish".into();
    }
    if tag.starts_with("pt_br") {
        return "Portuguese (Brazil)".into();
    }
    if tag.starts_with("pt") {
        return "Portuguese".into();
    }
    if tag.starts_with("ru") {
        return "Russian".into();
    }
    if tag.starts_with("it") {
        return "Italian".into();
    }
    if tag.starts_with("en") {
        return "English".into();
    }

    tag.split('_')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "English".into())
}

#[cfg(test)]
mod tests {
    use super::detect_output_language_from;

    fn detect(pairs: &[(&str, &str)]) -> String {
        detect_output_language_from(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    #[test]
    fn explicit_override_wins() {
        // 显式变量优先于一切，原样返回（去空白）。
        assert_eq!(
            detect(&[
                ("LANG", "ja_JP.UTF-8"),
                ("REVIEWGATE_OUTPUT_LANGUAGE", " Klingon ")
            ]),
            "Klingon"
        );
    }

    #[test]
    fn locale_precedence_lc_all_first() {
        // LC_ALL > LC_MESSAGES > LANG。
        assert_eq!(
            detect(&[
                ("LANG", "fr_FR.UTF-8"),
                ("LC_MESSAGES", "de_DE.UTF-8"),
                ("LC_ALL", "zh_CN.UTF-8"),
            ]),
            "Chinese (Simplified)"
        );
    }

    #[test]
    fn chinese_variants() {
        assert_eq!(detect(&[("LANG", "zh_CN.UTF-8")]), "Chinese (Simplified)");
        assert_eq!(detect(&[("LANG", "zh_TW.UTF-8")]), "Chinese (Traditional)");
        assert_eq!(detect(&[("LANG", "zh_HK")]), "Chinese (Traditional)");
    }

    #[test]
    fn common_locales_map() {
        assert_eq!(detect(&[("LANG", "ja_JP.UTF-8")]), "Japanese");
        assert_eq!(detect(&[("LANG", "ko_KR")]), "Korean");
        assert_eq!(detect(&[("LANG", "pt_BR.UTF-8")]), "Portuguese (Brazil)");
        assert_eq!(detect(&[("LANG", "pt_PT")]), "Portuguese");
        assert_eq!(detect(&[("LANG", "en_US.UTF-8")]), "English");
    }

    #[test]
    fn c_posix_and_empty_fallback_to_english() {
        assert_eq!(detect(&[("LANG", "C")]), "English");
        assert_eq!(detect(&[("LC_ALL", "POSIX")]), "English");
        // 空/全空白 → 视为未设置，回退英文。
        assert_eq!(detect(&[("LANG", "  ")]), "English");
        assert_eq!(detect(&[]), "English");
    }

    #[test]
    fn unknown_locale_uses_bare_tag() {
        // 未知但合法的语言标签：返回裸语言码（不硬塞英文）。
        assert_eq!(detect(&[("LANG", "vi_VN.UTF-8")]), "vi");
    }
}
