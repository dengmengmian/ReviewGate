# Scala Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [SCALA1] Do not call .get / .head on Option/Either/Try; unwrap safely with getOrElse, pattern matching, or fold.
- [SCALA2] Do not overuse implicit conversions/parameters in ways that cause ambiguity or surprising behavior; prefer given/using or explicit arguments and avoid over-broad scope.
- [SCALA3] Do not share mutable collections (mutable.Map/ListBuffer, etc.) across threads or Futures; use immutable collections or explicit synchronization.
- [SCALA4] Do not ignore Future failures; recover/onComplete or propagate them in compositions so exceptions are not silently dropped.
- [SCALA5] A match on a sealed trait/enum must cover all branches to avoid a runtime MatchError; do not mask missing cases with a catch-all case _.
- [SCALA6] Do not use var + mutable state in a loop to accumulate logic that map/fold can express, and note that == compares by value (equals), not by reference.
