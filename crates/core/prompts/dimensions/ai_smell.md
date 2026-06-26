Typical defects in AI-generated code. Checklist:
- Hallucination: calls to APIs, methods, fields, or libraries that do not exist.
- Plausible-looking but semantically wrong logic.
- Assumption drift: silently changing preconditions or postconditions that callers rely on.
- Overconfident boundary handling: using `unwrap`, non-null assertions, or casts to hide unhandled cases.
- Copy-paste without adaptation: code copied from elsewhere while variable names, types, or context were not fully updated. Use find_duplicate_functions for duplicate-function candidates.
- Placeholder or fake implementations: TODOs, empty implementations, or fake data returned as if it were real.
