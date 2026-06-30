//! CodeIndex 结果缓存装饰器。
//!
//! 一次审查里多个维度 Agent **并发**跑，且常重复查同一批改动符号（security/logic 都会去看
//! 同一个改动函数的定义/调用）。底层每次查询都要起 `git grep` 子进程 + 对候选文件做 AST 解析，
//! 重复查就是重复付费。这里在结果层做记忆化：同一 `(mode, symbol)` 第二次起直接命中缓存，
//! 跳过子进程与解析。错误不缓存（留待重试）。缓存只增不淘汰——一次审查生命周期短、符号集有界。

use super::{CodeIndex, Lang, SymbolLoc};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Mode {
    Definition,
    Callers,
    References,
}

/// 缓存表：`(mode, symbol)` → 共享只读结果。
type ResultCache = Mutex<HashMap<(Mode, String), Arc<Vec<SymbolLoc>>>>;

/// 给任意 [`CodeIndex`] 套一层结果缓存。
pub struct CachingIndex {
    inner: Arc<dyn CodeIndex>,
    cache: ResultCache,
}

impl CachingIndex {
    pub fn new(inner: Arc<dyn CodeIndex>) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, mode: Mode, symbol: &str) -> Option<Arc<Vec<SymbolLoc>>> {
        // 锁只在同步取/存的瞬间持有，绝不跨 await——避免阻塞其它并发 Agent。
        self.cache
            .lock()
            .unwrap()
            .get(&(mode, symbol.to_string()))
            .cloned()
    }

    fn put(&self, mode: Mode, symbol: &str, v: Arc<Vec<SymbolLoc>>) {
        self.cache
            .lock()
            .unwrap()
            .insert((mode, symbol.to_string()), v);
    }

    /// 命中即返回；否则调底层并写回。并发首次未命中可能重复计算一次——无正确性影响，可接受。
    async fn cached(&self, mode: Mode, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        if let Some(hit) = self.get(mode, symbol) {
            return Ok((*hit).clone());
        }
        let fresh = match mode {
            Mode::Definition => self.inner.find_definition(symbol, lang).await?,
            Mode::Callers => self.inner.find_callers(symbol, lang).await?,
            Mode::References => self.inner.find_references(symbol, lang).await?,
        };
        let arc = Arc::new(fresh);
        self.put(mode, symbol, arc.clone());
        Ok((*arc).clone())
    }
}

#[async_trait]
impl CodeIndex for CachingIndex {
    async fn find_definition(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.cached(Mode::Definition, symbol, lang).await
    }
    async fn find_callers(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.cached(Mode::Callers, symbol, lang).await
    }
    async fn find_references(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.cached(Mode::References, symbol, lang).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SymbolKind;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CodeIndex for Counting {
        async fn find_definition(
            &self,
            symbol: &str,
            _lang: Option<Lang>,
        ) -> Result<Vec<SymbolLoc>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![SymbolLoc {
                path: "a.rs".into(),
                line: 1,
                col: 1,
                kind: SymbolKind::Function,
                snippet: symbol.into(),
            }])
        }
        async fn find_callers(&self, _: &str, _: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok(vec![])
        }
        async fn find_references(&self, _: &str, _: Option<Lang>) -> Result<Vec<SymbolLoc>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn second_query_hits_cache() {
        let inner = Arc::new(Counting {
            calls: AtomicUsize::new(0),
        });
        let idx = CachingIndex::new(inner.clone());
        let a = idx.find_definition("foo", None).await.unwrap();
        let b = idx.find_definition("foo", None).await.unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // 底层只被调一次：第二次命中缓存。
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        // 不同符号不串缓存。
        idx.find_definition("bar", None).await.unwrap();
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn modes_do_not_collide() {
        let inner = Arc::new(Counting {
            calls: AtomicUsize::new(0),
        });
        let idx = CachingIndex::new(inner);
        // definition 命中缓存，但 callers 是不同 mode，不应复用 definition 的结果。
        assert_eq!(idx.find_definition("foo", None).await.unwrap().len(), 1);
        assert_eq!(idx.find_callers("foo", None).await.unwrap().len(), 0);
    }
}
