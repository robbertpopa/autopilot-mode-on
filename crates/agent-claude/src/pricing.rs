//! Token-based cost estimate for subscription steps.
//!
//! Prices are US dollars per million tokens from
//! <https://platform.claude.com/docs/en/about-claude/pricing>, read on
//! 2026-11-21. Update the table when that page changes. A model that is not
//! listed gets no estimate: a wrong price is worse than a missing one.

/// What one step consumed, as reported in the final `result` frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

/// Thousandths of a dollar per million tokens. Integers keep the arithmetic
/// exact: one unit per token is one thousandth of a micro-dollar.
struct Price {
    input: u64,
    short_write: u64,
    long_write: u64,
    read: u64,
    output: u64,
}

const fn price(input: u64, short_write: u64, long_write: u64, read: u64, output: u64) -> Price {
    Price {
        input,
        short_write,
        long_write,
        read,
        output,
    }
}

/// Longest names first, because a name is matched as a substring of the
/// model id and "opus-5" also occurs inside "opus-5-5".
const TABLE: &[(&str, Price)] = &[
    ("fable-5-2", price(8_000, 10_000, 16_000, 200, 40_000)),
    ("fable-5-1", price(10_000, 12_500, 20_000, 250, 50_000)),
    ("fable-5", price(10_000, 12_500, 20_000, 1_000, 50_000)),
    ("opus-5-5", price(4_000, 5_000, 8_000, 200, 20_000)),
    ("opus-5", price(5_000, 6_250, 10_000, 500, 25_000)),
    ("opus-4-8", price(5_000, 6_250, 10_000, 500, 25_000)),
    ("opus-4-7", price(5_000, 6_250, 10_000, 500, 25_000)),
    ("opus-4-6", price(5_000, 6_250, 10_000, 500, 25_000)),
    ("opus-4-5", price(5_000, 6_250, 10_000, 500, 25_000)),
    ("sonnet-5-5", price(2_000, 2_500, 4_000, 100, 10_000)),
    ("sonnet-5", price(2_000, 2_500, 4_000, 200, 10_000)),
    ("sonnet-4-6", price(3_000, 3_750, 6_000, 300, 15_000)),
    ("sonnet-4-5", price(3_000, 3_750, 6_000, 300, 15_000)),
    ("haiku-5-5", price(100, 125, 200, 10, 500)),
    ("haiku-4-5", price(1_000, 1_250, 2_000, 100, 5_000)),
];

/// Haiku 5.5 costs more for a request whose prompt exceeds this many tokens.
const HAIKU_LONG_PROMPT: u64 = 100_000;

/// Estimate the cost in millionths of a dollar, or `None` if the model is
/// unknown or the price cannot be determined from aggregate usage.
#[must_use]
pub fn estimate_microusd(model: &str, usage: Usage) -> Option<u64> {
    let model = model.to_ascii_lowercase().replace('.', "-");
    let (name, price) = TABLE.iter().find(|(name, _)| model.contains(name))?;
    let prompt = usage
        .input
        .saturating_add(usage.cache_read)
        .saturating_add(usage.cache_write_5m)
        .saturating_add(usage.cache_write_1h);
    // Usage is summed over every request of the run, but the long-prompt rate
    // applies per request. Above the threshold the split is unknowable.
    if *name == "haiku-5-5" && prompt > HAIKU_LONG_PROMPT {
        return None;
    }
    let parts = [
        (usage.input, price.input),
        (usage.cache_write_5m, price.short_write),
        (usage.cache_write_1h, price.long_write),
        (usage.cache_read, price.read),
        (usage.output, price.output),
    ];
    let mut thousandths: u64 = 0;
    for (tokens, rate) in parts {
        thousandths = thousandths.checked_add(tokens.checked_mul(rate)?)?;
    }
    // Round to the nearest micro-dollar.
    thousandths.checked_add(500).map(|total| total / 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pricing page's own worked example for Opus 5.5 with caching:
    // 10,000 uncached input, 40,000 cache reads, 15,000 output = $0.348.
    #[test]
    fn it_reproduces_the_documented_opus_example() {
        let usage = Usage {
            input: 10_000,
            cache_read: 40_000,
            output: 15_000,
            ..Usage::default()
        };
        assert_eq!(estimate_microusd("claude-opus-5-5", usage), Some(348_000));
    }

    #[test]
    fn each_token_class_uses_its_own_rate() {
        let one = |usage| estimate_microusd("claude-sonnet-5-5", usage);
        // Per million tokens: input $2, 5m write $2.50, 1h write $4, read $0.10, output $10.
        let million = 1_000_000;
        assert_eq!(
            one(Usage {
                input: million,
                ..Usage::default()
            }),
            Some(2_000_000)
        );
        assert_eq!(
            one(Usage {
                cache_write_5m: million,
                ..Usage::default()
            }),
            Some(2_500_000)
        );
        assert_eq!(
            one(Usage {
                cache_write_1h: million,
                ..Usage::default()
            }),
            Some(4_000_000)
        );
        assert_eq!(
            one(Usage {
                cache_read: million,
                ..Usage::default()
            }),
            Some(100_000)
        );
        assert_eq!(
            one(Usage {
                output: million,
                ..Usage::default()
            }),
            Some(10_000_000)
        );
    }

    // Invariant: the longer model name wins. If "opus-5" matched first, an
    // Opus 5.5 run would be priced at the Opus 5 rate.
    #[test]
    fn specific_versions_beat_their_prefixes_and_dates_are_ignored() {
        let usage = Usage {
            input: 1_000_000,
            ..Usage::default()
        };
        assert_eq!(
            estimate_microusd("claude-opus-5-5-20260601", usage),
            Some(4_000_000)
        );
        assert_eq!(estimate_microusd("claude-opus-5", usage), Some(5_000_000));
        assert_eq!(
            estimate_microusd("claude-sonnet-5-5", usage),
            Some(2_000_000)
        );
        assert_eq!(
            estimate_microusd("Claude-Sonnet-4.6", usage),
            Some(3_000_000)
        );
    }

    #[test]
    fn unknown_models_and_ambiguous_long_haiku_prompts_get_no_estimate() {
        let usage = Usage {
            input: 1_000,
            ..Usage::default()
        };
        assert_eq!(estimate_microusd("haiku", usage), None);
        assert_eq!(estimate_microusd("claude-opus-4-1", usage), None);
        assert_eq!(estimate_microusd("claude-haiku-5-5", usage), Some(100));
        let long = Usage {
            input: 100_001,
            ..Usage::default()
        };
        assert_eq!(estimate_microusd("claude-haiku-5-5", long), None);
    }

    #[test]
    fn huge_counts_do_not_overflow() {
        let usage = Usage {
            input: u64::MAX,
            ..Usage::default()
        };
        assert_eq!(estimate_microusd("claude-opus-5-5", usage), None);
    }
}
