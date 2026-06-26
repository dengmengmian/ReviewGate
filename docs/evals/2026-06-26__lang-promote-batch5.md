# 第五批语言验证(最后16种) + r/ruby trivial重测 — rules_dir 注入

## crystal/bad — decision: block
    · 运行时崩溃：`first_word` 被调用时参数为 `nil`（第6行），`line.not_nil!` 会对 `nil` 调用 `.not_nil!`，在 

## crystal/ok — decision: pass

## cangjie/bad — decision: block
    · [CJ1] 对 `Option<String>` 直接调用 `getOrThrow()` 而未先检查是否为 `Some`。`main()` 中传入 `None`

## cangjie/ok — decision: pass

## shell/bad — decision: block
    · 缺少对 `$1` 的空值校验。当脚本无参数调用时，`TARGET` 为空字符串，`$TARGET/build` 展开为 `/build`，导致 `rm -rf 
    · 空变量危险操作：当 `$TARGET` 为空或未设置时，`rm -rf $TARGET/build` 将展开为 `rm -rf /build`，会尝试删除根目录

## shell/ok — decision: block
    · `set -euo pipefail` 中的 `-u`（nounset）导致无参数调用时脚本在第 4 行直接崩溃（`$1: unbound variable`）
    · 【复制粘贴未适配】脚本文件名为 `src.sh`，但 usage 信息中硬编码了 `clean.sh`，很可能从 `clean.sh` 复制而来但未更新文件名引

## powershell/bad — decision: block
    · [PS3] $null 比较放在右侧：`$items -eq $null` 在 `$items` 为数组时返回过滤后的集合而非布尔值。当传入空数组 `@()` 
    · 函数名 `Test-Empty` 暗示测试\

## powershell/ok — decision: warn
    · [AI Smell] 函数名 `Test-Empty` 暗示检测\

## html/bad — decision: warn
    · [HTML1] 用户可控内容通过 `| raw` 过滤器直接输出到页面，未进行 HTML 转义，构成存储型/反射型 XSS 漏洞。攻击者可注入任意 JavaSc
    · AI 生成占位/示例代码：HTML 注释直接标注了自身安全漏洞（HTML1 XSS、HTML2 缺失 alt），读起来像是 AI 对\

## html/ok — decision: pass

## css/bad — decision: pass

## css/ok — decision: pass

## vue/bad — decision: block
    · [AI-Smell] 占位/不完整实现：`<template>` 中引用了 `items` 和 `item.userHtml`，但缺少 `<script>` 段
    · [VUE1] `v-html` 直接渲染用户可控内容 `item.userHtml`，存在 XSS 漏洞。攻击者可以注入恶意脚本（如 `<img src=x o

## vue/ok — decision: block
    · 组件缺少 `<script>` 块：模板中引用了 `items`、`item.id`、`item.text`，但整个文件仅包含 `<template>` 块，未

## svelte/bad — decision: block
    · SVELTE1: `{@html comment}` 将用户控制的 `comment` prop 作为原始 HTML 直接渲染，构成 XSS 漏洞。`{@htm

## svelte/ok — decision: pass

## sql/bad — decision: warn
    · 【业务】无 WHERE 条件的批量状态更新会将整个 users 表的所有用户设为 inactive，导致全员无法登录，属于严重业务事故。缺少必要的业务护栏：未限
    · AI 生成气味 — 占位/虚假实现：该 SQL 文件包含一个缺少 WHERE 子句的 `UPDATE` 语句，会将整张 users 表的所有行状态设为 'ina

## sql/ok — decision: pass

## graphql/bad — decision: warn
    · [DoS] 自递归类型 `User.friends` 无查询深度限制或分页上限，攻击者可构造深度嵌套查询（如 `users { friends { friend

## graphql/ok — decision: block
    · [GQL1] Schema 存在循环引用（User → FriendConnection.edges → User），允许查询无限嵌套深度。即使每层有分页参数 
    · [AI-Smell] 分页参数与返回类型不一致：`users` 查询使用了游标分页参数（`first`、`after`），但返回类型是普通列表 `[User!]

## solidity/bad — decision: block
    · [AI Smell] 语义不完整：`balances` 映射从未被任何函数写入（没有 deposit/receive/fallback），且合约缺少 `rece
    · [SOL1] 重入攻击漏洞：`withdraw()` 函数在更新 `balances[msg.sender]` 之前执行了外部调用 `msg.sender.ca

## solidity/ok — decision: pass

## fortran/bad — decision: block
    · [FORT3] 未初始化变量 `total`：在第 4 行 `total = total + i` 中，`total` 在被读取之前从未赋初值。由于缺少 `im

## fortran/ok — decision: pass

## cobol/bad — decision: block
    · [COB1] 计算结果 100 超出 PIC 99 的容量（最大值 99），高位被截断，实际存入 WS-TOTAL 的值为 00 而非 100。缺少 ON SI

## cobol/ok — decision: pass

## pascal/bad — decision: block
    · 资源泄漏：第 6 行 `New(p)` 分配的堆内存从未通过 `Dispose(p)` 释放。程序在第 9 行 `end.` 处退出，缺少对应的释放路径。虽然进

## pascal/ok — decision: pass

## dockerfile/bad — decision: block
    · [DOCK2] 容器以 root 用户运行，缺少 `USER` 指令来降权。如果应用存在漏洞，攻击者可直接获得 root 权限，增加容器逃逸和横向移动风险。（另
    · [DOCK5] `COPY . .` 缺少 `.dockerignore` 文件，可能将 `.git`（含提交历史）、`.env`（密钥）、本地配置文件等敏感内

## dockerfile/ok — decision: pass

## terraform/bad — decision: pass

## terraform/ok — decision: pass

## r/ok3 — decision: pass

## ruby/ok3 — decision: pass

