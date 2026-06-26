# Vue Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [VUE1] `v-html` rendering user-controlled content constitutes XSS.
- [VUE2] `v-for` missing a stable and unique `:key`, or using index as the key, causes list reuse misalignment and state crosstalk.
- [VUE3] A child component directly mutating `props` violates one-way data flow and the change is not propagated back to the parent (use emit or a local copy).
- [VUE4] Destructuring `reactive`/`props` or reassigning a reactive object wholesale loses reactivity so the view does not update (use `toRefs`/`storeToRefs`).
- [VUE5] Timers, subscriptions, or event listeners created in `watch`/`watchEffect` that are not cleaned up on stop or `onUnmounted` cause leaks and repeated triggering.
