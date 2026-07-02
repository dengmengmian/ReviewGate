//! ReviewGate core engine.
//!
//! 给 AI 生成的代码加一道合并前质检：优先暴露高风险问题，折叠低置信噪音。
//!
//! 所有智能都在这个 crate 里：LLM 客户端、Agent 循环、多维并行、行号重定位、
//! 证伪 Judge、闸口逻辑。CLI / Claude Skill / GitHub Action 都是它的薄包装。

pub mod agent;
pub mod apply;
pub mod config;
pub mod diff;
pub mod gate;
pub mod github;
pub mod index;
pub mod judge;
pub mod language;
pub mod llm;
pub mod model;
pub mod progress;
pub mod relocate;
pub mod review;
pub mod tool;

/// Crate version, surfaced via `reviewgate --version`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_manifest() {
        let manifest = env!("CARGO_PKG_VERSION");
        assert_eq!(version(), manifest);
        assert!(!version().is_empty());
    }
}

// 后续里程碑逐步加入的模块（M1.3 起）：
// pub mod llm;       // LlmClient trait + Anthropic / OpenAI 兼容
// pub mod diff;      // git diff 获取 + hunk 解析
// pub mod tool;      // 工具集 trait + read_file / code_search / find_file
// pub mod index;     // CodeIndex: GrepIndex(v0) / TreeSitterIndex(v1)
// pub mod agent;     // Agent tool-use 循环 + 多维编排
// pub mod relocate;  // 行号重定位三级匹配
// pub mod judge;     // 证伪验证 + 去重 + 置信度聚合
// pub mod gate;      // 闸口：阈值 → pass/warn/block
// pub mod config;    // 配置加载
