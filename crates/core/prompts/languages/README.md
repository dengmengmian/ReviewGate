# ReviewGate Starter Language Rule Library

These are out-of-the-box language trap templates, not organization-specific conventions.

- After copying them into `.reviewgate/rules/`, `rules_dir = ".reviewgate/rules"` injects the matching `<language>.md` only when that language is changed.
- These rules only steer review focus: report only when the diff clearly hits a rule and creates real risk.
- Organization-specific naming, logging, exception, and authorization-boundary rules should still live in `.reviewgate/rules/business.md` or your own maintained `<language>.md` files.
