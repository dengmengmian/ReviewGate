# 召回打磨复测: csharp(空catch +CS6) / terraform(TF1/TF2 加强)

## csharp_bad(应抓) — decision: block
    · `DoWork()` 方法体为空，是一个明显的占位符/假实现。类名 `S` 毫无意义，结合空实现与 `catch {}` 静默吞异常的模式，强烈表明这是 AI 
    · 空的 `catch { }` 块静默吞噬了所有异常。逻辑检查清单明确要求报告\

## csharp_clean(应PASS) — decision: warn
    · `Log` 方法为空实现（placeholder/fake implementation）。方法签名看起来像是在记录异常日志，但方法体为空，实际上不会产生任何日

## terraform_bad_1(应抓) — decision: block
    · 数据库密码 \
    · 安全组将 SSH（端口 22）对 0.0.0.0/0 开放，允许来自互联网任意 IP 的 SSH 访问，存在暴力破解和未授权访问风险。
    · AI 生成的典型占位密码：`\

## terraform_bad_2(determinism) — decision: block
    · [TF2] SSH 端口对全球开放：安全组 `aws_security_group.ssh` 的入站规则将 22 端口（SSH）的 `cidr_blocks` 
    · 数据库密码 `S3cr3tP@ss` 以明文硬编码在变量 `default` 中，会暴露在版本控制和 Terraform 状态文件中。（另由 business 
    · 安全组将 SSH 端口 (22) 对全网开放 (`0.0.0.0/0`)，允许任意 IP 尝试连接，存在暴力破解风险。

## terraform_clean(应PASS) — decision: block
    · AI 幻觉/占位实现：`aws_security_group` 资源缺少必填参数 `name`（或 `name_prefix`）。在当前状态下，`terrafo
    · `admin_cidr` 变量控制 SSH（端口 22）访问范围，但缺少输入验证（validation block）。调用者可以传入 `\
    · 变量 `db_password` 已声明但从未被任何资源或输出引用，属于死代码。这可能意味着数据库相关资源的实现不完整，或者该变量被遗漏而未连接到实际的数据库资


## 加强后·无懈可击 clean 复测(精度须保持)
- csharp_clean2 — decision: pass
- terraform_clean2 — decision: pass

## 真实实质性提交(默认全规则): ripgrep 43e2f08 (gitignore 修复)
- decision: pass
    · rules_dir does not exist or is not readable: .reviewgate/rules
    · 墙钟超时，该维度未审完（已保留其部分发现）
