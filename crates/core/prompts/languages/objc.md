# Objective-C Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [OBJC1] Blocks and delegates holding `self` easily create a retain cycle; inside closures use `__weak typeof(self) weakSelf`.
- [OBJC2] Do not rely on sending a message to `nil` "safely returning 0/nil" to mask logic errors; behavior is undefined for struct/float returns, and it silently swallows branches that should have been handled.
- [OBJC3] Delegate and block properties should be `weak`/`copy` rather than `strong`; a `strong` block property extends the lifetime of its captured objects.
- [OBJC4] KVO/`NSNotification` registered via `addObserver` must be deregistered in `dealloc` or at the appropriate time, otherwise notifying a deallocated object crashes.
- [OBJC5] Mutable properties accessed from multiple threads should be `atomic` or locked; also note `atomic` does not guarantee thread safety for compound operations.
- [OBJC6] Do not hold mutable values like `NSString`/`NSArray` with `assign`/`unsafe_unretained`, or `copy` an `NSMutable` container and then still use it as mutable.
