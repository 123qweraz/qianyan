use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::config_manager::UserDictData;
use crate::Config;

use super::{
    Candidate,
};

pub trait Filter: Send + Sync {
    fn filter(
        &self,
        input: &str,
        candidates: Vec<Candidate>,
        config: &Config,
        context: Option<&str>,
        context_pair: Option<(&str, &str)>,
    ) -> Vec<Candidate>;
}

/// 匹配层级评分过滤器：统一 match_level 评分，替代 ChineseScheme::post_process 的独立评分
/// 排序规则：精确匹配 (level 3) > 模糊/简拼 (level 2) > 前缀 (level 1)，同层内按 weight 降序
pub struct MatchLevelScoringFilter;
impl Filter for MatchLevelScoringFilter {
    fn filter(
        &self,
        input: &str,
        mut candidates: Vec<Candidate>,
        config: &Config,
        _context: Option<&str>,
        _context_pair: Option<(&str, &str)>,
    ) -> Vec<Candidate> {
        let input_syllables = estimate_syllables(input);

        for c in &mut candidates {
            let base = match c.match_level {
                3 => 30_000_000.0 + config.input.ranking.exact_match_bonus,
                2 => 20_000_000.0,
                1 => 10_000_000.0,
                _ => 0.0,
            };
            let char_count = c.simplified.chars().count() as f64;
            let len_diff = (char_count - input_syllables as f64).max(0.0);
            let penalty = if c.match_level == 2 {
                len_diff * 10000.0
            } else {
                len_diff * 1000.0
            };
            c.weight = base + c.weight - penalty;
        }

        candidates.sort_by(|a, b| {
            b.match_level
                .cmp(&a.match_level)
                .then(b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal))
        });
        candidates
    }
}

fn estimate_syllables(input: &str) -> usize {
    if input.is_empty() {
        return 0;
    }
    input.chars().filter(|&c| c == ' ' || c == '\'' || c == ';').count() + 1
}

/// 繁简转换过滤器
pub struct TraditionalFilter;
impl Filter for TraditionalFilter {
    fn filter(
        &self,
        _input: &str,
        mut candidates: Vec<Candidate>,
        config: &Config,
        _context: Option<&str>,
        _context_pair: Option<(&str, &str)>,
    ) -> Vec<Candidate> {
        if config.input.enable_traditional {
            for c in &mut candidates {
                c.text = c.traditional.clone();
            }
        } else {
            for c in &mut candidates {
                c.text = c.simplified.clone();
            }
        }
        candidates
    }
}

/// 动态自适应过滤器 (上下文联想)
pub struct AdaptiveFilter {
    pub ngram_history: Arc<ArcSwap<UserDictData>>,
    pub profile: String,
}

impl AdaptiveFilter {
    pub fn new(
        ngram_history: Arc<ArcSwap<UserDictData>>,
        profile: String,
    ) -> Self {
        Self {
            ngram_history,
            profile,
        }
    }
}

impl Filter for AdaptiveFilter {
    fn filter(
        &self,
        _input: &str,
        mut candidates: Vec<Candidate>,
        config: &Config,
        context: Option<&str>,
        context_pair: Option<(&str, &str)>,
    ) -> Vec<Candidate> {
        if config.input.enable_context_sorting {
            let ngram_guard = self.ngram_history.load();
            if let Some(profile_ngram) = ngram_guard.get(&self.profile) {
                // bigram: key=last_word
                if let Some(ctx) = context {
                    if let Some(entries) = profile_ngram.get(ctx) {
                        let ngram_map: std::collections::HashMap<String, u32> =
                            entries.iter().map(|(w, c)| (w.clone(), *c)).collect();
                        for c in &mut candidates {
                            if let Some(&count) = ngram_map.get(c.simplified.as_ref()) {
                                let effective = count.min(40) as f64;
                                c.weight += effective * 50_000_000.0;
                            }
                        }
                    }
                }

                // trigram: key="prev2|prev1"
                if let Some((prev2, prev1)) = context_pair {
                    let trigram_key = format!("{}|{}", prev2, prev1);
                    if let Some(entries) = profile_ngram.get(&trigram_key) {
                        for c in &mut candidates {
                            if let Some(&count) = entries.iter()
                                .find(|(w, _)| w == c.simplified.as_ref())
                                .map(|(_, c)| c)
                            {
                                let effective = count.min(40) as f64;
                                c.weight += effective * 60_000_000.0;
                            }
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    }
}
