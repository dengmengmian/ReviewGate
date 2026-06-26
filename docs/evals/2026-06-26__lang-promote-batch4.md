# 第四批语言验证(clojure/groovy/julia/r/ocaml/fsharp/zig/nim) + erlang/ruby干净重测

## clojure/bad — decision: warn
    · Java 互操作调用 `(.length s)` 在 `s` 为 `nil` 时会抛出 `NullPointerException`。Clojure 中 `nil` 是合

## clojure/ok — decision: pass

## groovy/bad — decision: pass

## groovy/ok — decision: pass

## julia/bad — decision: pass

## julia/ok — decision: pass

## r/bad — decision: block
    · [R3] 使用 `== NA` 检测缺失值，该写法永远无法正确工作。在 R 中，`x == NA` 对任何 x（包括 NA 本身）都返回 `NA`，而非 `TRUE`/`

## r/ok — decision: block
    · 当 `x` 为空向量（如 `numeric(0)`）时，`is.na(x)` 返回 `logical(0)`（零长度逻辑向量）。R 中 `if (logical(0))`

## ocaml/bad — decision: warn
    · [边界条件] `List.hd` 在空列表上抛出 `Failure \

## ocaml/ok — decision: pass

## fsharp/bad — decision: warn
    · `List.head` 在空列表上会抛出 `System.ArgumentException`。函数签名 `int list -> int` 暗示总能返回一个 int，但

## fsharp/ok — decision: pass

## zig/bad — decision: pass

## zig/ok — decision: pass

## nim/bad — decision: pass

## nim/ok — decision: pass

## erlang/ok2 — decision: pass

## ruby/ok2 — decision: block
    · `Hash#fetch(:price, 0)` 仅在键缺失时使用默认值，当键存在但值为 `nil` 时仍返回 `nil`。例如输入 `[{price: 10}, {pri

