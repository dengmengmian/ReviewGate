# 第三批语言验证(scala/dart/objc/lua/perl/haskell/elixir/erlang + ruby重测) — rules_dir 注入

## scala/bad — decision: block
    · [SCALA1] 对 `Option` 直接调用 `.get`，当传入 `None` 时将抛出 `NoSuchElementException`，导致运行时崩溃。边界模拟

## scala/ok — decision: pass

## dart/bad — decision: block
    · 函数签名声明接受可空参数 `String? s`，但函数体使用空断言 `s!` 直接访问 `length`。当传入 `null` 时，`s!` 会在运行时抛出 `Type

## dart/ok — decision: pass

## objc/bad — decision: block
    · retain cycle：block 强捕获 self，而 block 又通过实例变量 _b 被 self 强持有，形成 self → _b → block → self

## objc/ok — decision: pass

## lua/bad — decision: block
    · [LUA1] 变量 `x` 未使用 `local` 声明，被隐式写入全局表 `_G`，会造成跨模块状态泄漏。函数 `f()` 每次调用都会修改全局 `x`，其他模块若引用

## lua/ok — decision: pass

## perl/bad — decision: pass

## perl/ok — decision: pass

## haskell/bad — decision: block
    · [边界条件] head 是部分函数，对空列表 [] 会抛出运行时异常，但函数签名 [Int] -> Int 承诺处理任意列表。输入 [] 时程序崩溃。（另由 busine
    · 过度自信的边界处理：`head` 在空列表上会抛出运行时异常，但函数类型签名 `[Int] -> Int` 承诺对任意列表返回 `Int`。这是 AI 生成代码中常见的「

## haskell/ok — decision: pass

## elixir/bad — decision: warn
    · 使用 String.to_atom/1 将参数直接转为 atom。atom 在 Erlang VM 中不被 GC，若 s 来自外部输入（如 HTTP 参数、用户输入），攻

## elixir/ok — decision: pass

## erlang/bad — decision: block
    · [ERL3] `catch do_work()` 捕获了所有异常并将其转为返回值，这违反了 let-it-crash 原则。`catch` 捕获包括 `exit` 信号在

## erlang/ok — decision: block
    · 异常类型被静默转换且调用栈丢失。`_:E` 匹配所有异常类型（error/exit/throw），但 `erlang:raise(error, E, [])` 总是以 `
    · 整体代码呈现 AI 生成样板特征：（1）模块名 `m` 过于通用，像是模板占位符；（2）`do_work() -> ok.` 是典型的空占位实现；（3）`run/0` 中

## ruby/bad — decision: block
    · bare `rescue`（无异常类型限定）会静默吞掉 `do_work` 抛出的所有 `StandardError` 子类异常。调用方 `run` 永远看起来像成功执行

## ruby/ok — decision: block
    · 缺少对 `:price` 键缺失的处理：当 `items` 数组中某个哈希不含 `:price` 键时，`i[:price]` 返回 `nil`，导致 `Array#su

