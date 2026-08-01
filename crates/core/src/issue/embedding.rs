//! 本地可复现 embedding：特征哈希，离线可用；失败时上层可降级。

use super::store::{bytes_to_f32s, f32s_to_bytes};

pub const EMBED_DIM: usize = 128;
pub const EMBED_MODEL: &str = "reviewgate-local-hash";
pub const EMBED_VERSION: &str = "1";

/// 将文本嵌入为固定维度向量（确定性，跨进程稳定）。
pub fn embed_local(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; EMBED_DIM];
    let lower = text.to_ascii_lowercase();
    // unigram tokens
    for token in lower.split(|c: char| !c.is_alphanumeric()) {
        if token.len() < 2 {
            continue;
        }
        let h = hash_token(token);
        let idx = (h as usize) % EMBED_DIM;
        v[idx] += 1.0;
        // bigram char features for short errors
        if token.len() >= 3 {
            let bytes = token.as_bytes();
            for w in bytes.windows(3) {
                let h2 = hash_bytes(w);
                let idx2 = (h2 as usize) % EMBED_DIM;
                v[idx2] += 0.5;
            }
        }
    }
    // L2 normalize
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

pub fn embed_local_bytes(text: &str) -> Vec<u8> {
    f32s_to_bytes(&embed_local(text))
}

pub fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes_to_f32s(bytes)
}

fn hash_token(s: &str) -> u64 {
    hash_bytes(s.as_bytes())
}

fn hash_bytes(b: &[u8]) -> u64 {
    // FNV-1a 64
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 可注入的 embedding 后端。
pub trait Embedder: Send + Sync {
    fn model(&self) -> &str;
    fn version(&self) -> &str;
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
}

/// 默认本地 embedder。
#[derive(Debug, Default, Clone)]
pub struct LocalEmbedder;

impl Embedder for LocalEmbedder {
    fn model(&self) -> &str {
        EMBED_MODEL
    }
    fn version(&self) -> &str {
        EMBED_VERSION
    }
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(embed_local(text))
    }
}

/// 强制失败的 embedder（测试降级路径）。
#[derive(Debug, Default, Clone)]
pub struct FailingEmbedder;

impl Embedder for FailingEmbedder {
    fn model(&self) -> &str {
        "failing"
    }
    fn version(&self) -> &str {
        "0"
    }
    fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        anyhow::bail!("embedding forced failure")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::store::cosine_similarity;

    #[test]
    fn similar_texts_have_higher_cosine() {
        let a = embed_local("windows save crash access violation");
        let b = embed_local("access violation when saving on windows");
        let c = embed_local("documentation typo in readme file");
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ac = cosine_similarity(&a, &c);
        assert!(sim_ab > sim_ac, "sim_ab={sim_ab} sim_ac={sim_ac}");
        assert_eq!(a.len(), EMBED_DIM);
    }
}
