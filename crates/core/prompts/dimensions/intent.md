Intent / technical review — does the implementation fulfill the stated intent? Checklist:
- Missing requirements: acceptance criteria, required branches, or call-site updates the intent demands but the code omits. Reverse-check with tools before reporting a gap.
- Intent deviation: the implementation does the wrong thing or misreads the requirement.
- Breaking changes: existing behavior, callers, or contracts broken without the intent calling for it.
- Approach risk / better alternative: design or maintainability concern, or a clearly better technical approach (suggestion-level, lower confidence).
