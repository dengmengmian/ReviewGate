# Solidity Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SOL1] State not updated before an external call/transfer violates checks-effects-interactions and risks a reentrancy attack.
- [SOL2] Arithmetic without SafeMath below 0.8 can overflow/underflow integers.
- [SOL3] Using `tx.origin` for authorization can be bypassed by phishing through an intermediary contract; use `msg.sender`.
- [SOL4] `call`/`send`/low-level calls whose return value is not checked silently ignore failures.
- [SOL5] Generating randomness from `block.timestamp`, `blockhash`, or block variables can be predicted or manipulated by miners/callers.
- [SOL6] A `delegatecall` to an untrusted target or with mismatched storage layout can be hijacked to overwrite the caller's state.
