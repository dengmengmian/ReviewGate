# 第二批语言验证(C#/Ruby/PHP/Swift/Kotlin) — 候选规则经 rules_dir 注入

## csharp/bad — decision: pass

## csharp/ok — decision: pass

## ruby/bad — decision: block
    · [RB1] 裸 `rescue` 静默吞掉所有异常。`do_work` 中任何异常都会被捕获且丢弃，调用方完全不知道执行是否成功。故障被隐藏后，后续逻辑可能在错误状态下继续执行，导
    · 空实现占位符：`do_work` 方法体为空，无 TODO 注释或任何指示说明这是有意留空的桩代码。该方法被 `run` 调用，但没有任何实际逻辑，属于典型的 AI 生成骨架代码中

## ruby/ok — decision: warn
    · 无意义的 rescue-re-raise 空操作：捕获所有异常后原样重新抛出，整个过程未做任何处理。这是典型的 AI 生成样板代码——看似添加了\
    · `do_work` 方法是空实现（占位符/stub），不执行任何实际操作。如果这是 AI 生成的代码，空方法体表明生成内容不完整，可能遗漏了核心业务逻辑。如果是有意留下的占位符，建

## php/bad — decision: block
    · [PHP1] 使用弱类型比较 `==` 存在类型绕过风险。PHP 的类型戏法可能导致非预期的相等判断，例如 `check('0e123', '0e456')` 或 `check('

## php/ok — decision: pass

## swift/bad — decision: block
    · [ai_smell] 过度自信的边界处理：`a.first!` 对空数组会触发运行时崩溃。AI 使用强制解包 `!` 掩盖了 `first` 返回 `nil` 的边界情况，函数签名
    · 对空数组调用 `a.first!` 会触发强制解包 nil 值，导致运行时崩溃（`Fatal error: Unexpectedly found nil while unwrapp

## swift/ok — decision: pass

## kotlin/bad — decision: block
    · [KT1] 函数 `len` 接受可空参数 `s: String?`，但内部使用 `!!` 强制解包。调用者传入 `null` 时将抛出 `NullPointerException

## kotlin/ok — decision: pass

