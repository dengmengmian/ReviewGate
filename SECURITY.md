# 安全策略 / Security Policy

## 报告漏洞 / Reporting a Vulnerability

请**不要**通过公开 issue 报告安全漏洞。
Please do **not** report security vulnerabilities through public issues.

优先使用 GitHub 的私密上报：仓库 **Security → Report a vulnerability**（Private vulnerability reporting）。
Prefer GitHub private reporting: **Security → Report a vulnerability** on this repo.

我们会尽快确认并修复；修复发布后会在 release notes 致谢（如你愿意）。
We will acknowledge and fix as soon as possible, and credit you in the release notes (if you wish).

## 范围 / Scope

ReviewGate 是**只读**质量闸口：不写文件、不执行任意命令。请特别关注：
ReviewGate is a **read-only** quality gate. Of particular interest:

- 路径限制绕过（读取仓库外文件）/ path-confinement bypass (reading files outside the repo)
- `--exec-verify` 沙箱逃逸（该模式为 opt-in 弱隔离，仅限可信环境）/ `--exec-verify` sandbox escape (opt-in, weakly isolated, trusted environments only)
- 终端转义注入（LLM 内容渲染）/ terminal-escape injection via rendered LLM content
- 凭据/密钥泄露路径 / credential or secret leakage paths
