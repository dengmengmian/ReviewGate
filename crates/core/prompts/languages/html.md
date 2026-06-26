# HTML Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [HTML1] User-controllable content is inserted into the page without escaping, constituting stored/reflected XSS.
- [HTML2] Images lack `alt`, and form controls lack an associated `label`/`aria-label`, breaking accessibility and screen readers.
- [HTML3] A state-changing form (POST) lacks a CSRF token or protection field.
- [HTML4] A `target="_blank"` link lacks `rel="noopener noreferrer"`, enabling reverse tabnabbing.
- [HTML5] Inline event handlers (`onclick=` etc.) or inline `javascript:` are used, violating CSP and easily becoming XSS injection points.
