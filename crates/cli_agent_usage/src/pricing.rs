//! Estimated USD pricing per model. Rates are USD per 1,000,000 tokens.
//! ESTIMATE ONLY — maintain against published pricing. Matching is by substring so
//! version suffixes (claude-opus-4-7 / -4-8, gpt-5 / gpt-5.5 / gpt-5-codex) all resolve.

use crate::TokenCounts;

#[derive(Clone, Copy)]
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

fn rates(model: &str) -> Option<Rates> {
    let m = model.to_ascii_lowercase();
    // Claude
    if m.contains("opus") {
        return Some(Rates {
            input: 15.0,
            output: 75.0,
            cache_read: 1.50,
            cache_write: 18.75,
        });
    }
    if m.contains("sonnet") {
        return Some(Rates {
            input: 3.0,
            output: 15.0,
            cache_read: 0.30,
            cache_write: 3.75,
        });
    }
    if m.contains("haiku") {
        return Some(Rates {
            input: 0.80,
            output: 4.0,
            cache_read: 0.08,
            cache_write: 1.0,
        });
    }
    if m.contains("fable") {
        // Placeholder until Fable 5 pricing is published; sonnet-class estimate.
        return Some(Rates {
            input: 3.0,
            output: 15.0,
            cache_read: 0.30,
            cache_write: 3.75,
        });
    }
    // Codex / OpenAI (gpt-5 family, incl. gpt-5-codex, gpt-5.5)
    if m.contains("gpt-5") || m.contains("gpt5") || m.contains("codex") {
        return Some(Rates {
            input: 1.25,
            output: 10.0,
            cache_read: 0.125,
            cache_write: 0.0,
        });
    }
    None
}

pub fn cost(model: &str, t: &TokenCounts) -> f64 {
    let Some(r) = rates(model) else {
        // Unknown model (e.g. Claude's "<synthetic>" pseudo-model) contributes 0 cost.
        // Silent by design: a library must not write to stderr; the app layer can log this.
        return 0.0;
    };
    let per = 1_000_000.0;
    (t.input as f64) / per * r.input
        + (t.output as f64) / per * r.output
        + (t.cache_read as f64) / per * r.cache_read
        + (t.cache_write as f64) / per * r.cache_write
}

#[cfg(test)]
mod tests {
    use super::cost;
    use crate::TokenCounts;

    #[test]
    fn opus_priced_per_mtok() {
        // 1M input @ $15, 1M output @ $75, 1M cache_read @ $1.50, 1M cache_write @ $18.75
        let t = TokenCounts {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
        };
        let c = cost("claude-opus-4-8", &t);
        assert!((c - (15.0 + 75.0 + 1.50 + 18.75)).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn codex_gpt5_priced() {
        // 1M input @ $1.25, 1M output @ $10, 1M cache_read @ $0.125
        let t = TokenCounts {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
        };
        let c = cost("gpt-5.5", &t);
        assert!((c - (1.25 + 10.0 + 0.125)).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn unknown_model_is_zero_cost() {
        let t = TokenCounts {
            input: 1_000_000,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        };
        assert_eq!(cost("totally-unknown-model", &t), 0.0);
    }
}
