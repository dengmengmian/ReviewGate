# 收尾批：shell/powershell/vue/graphql 干净样例重测

## shell/ok — decision: pass

## powershell/ok — decision: pass

## vue/ok — decision: pass

## graphql/ok — decision: block
    · [GQL1] `first` 分页参数为无上限的 `Int!`，攻击者可传入极大值（如 `first: 2147483647`）导致服务端一次性查询海量数据，造成内存耗尽
    · [AI-Hallucination] Relay Connection 模式实现不完整：命名采用了 Relay 约定（`XxxConnection`、`edges`、`p

