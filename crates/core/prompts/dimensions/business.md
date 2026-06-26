Project business rules and domain semantics, using the injected "Project business rules" section above as the source of truth. Checklist:
- Whether the change clearly violates any project business rule, or whether a new/modified path bypasses such a rule.
- Authorization boundaries: user-owned resources must validate ownership (`owner_id`, tenant isolation); admin/backend endpoints must validate roles.
- State machines: transitions must be legal and must check required prior states and terminal states.
- Critical numbers such as money, inventory, orders, and counters: units must be consistent, with no overflow, races, or duplicate deductions.
- Report only clear rule violations or bypasses. Leave generic bugs and style issues to other dimensions.
