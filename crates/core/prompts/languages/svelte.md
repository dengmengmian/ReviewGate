# Svelte Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SVELTE1] `{@html ...}` rendering user-controlled content constitutes XSS.
- [SVELTE2] Putting side-effecting code (requests, assignment chains) in a reactive statement `$:` lets dependency tracking trigger it repeatedly or unexpectedly.
- [SVELTE3] Manually subscribing to a store (`store.subscribe`) without storing and calling the unsubscribe function, or subscribing in a non-component context without cleanup, leaks memory.
- [SVELTE4] An `export let x = ...` default does not take effect when the parent passes `undefined`, or sharing an object/array default across instances causes state crosstalk.
- [SVELTE5] Directly mutating an array/object (`arr.push`, etc.) without reassigning cannot be detected by Svelte's compile-time reactivity, so the view does not update.
