use std::collections::HashMap;

use crate::tree::{Group, SymbolKey};

/// Delta info for a single path.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    pub before: Option<u64>,
    pub after: Option<u64>,
}

impl Delta {
    pub fn diff_bytes(&self) -> i64 {
        self.after.unwrap_or(0) as i64 - self.before.unwrap_or(0) as i64
    }

    pub fn diff_pct(&self) -> f64 {
        if self.after.is_none() && self.before.is_some() {
            // Removed: use a sentinel so delta_color() can render it as bright red
            // ("removed") rather than folding it into the green "shrank 100%" case.
            return f64::NEG_INFINITY;
        }

        let b = self.before.unwrap_or(0) as f64;
        if b == 0.0 {
            if self.after.unwrap_or(0) > 0 { f64::INFINITY } else { 0.0 }
        } else {
            self.diff_bytes() as f64 / b * 100.0
        }
    }
}

pub fn compute_diff<K: std::hash::Hash + Eq + Clone>(
    before: &HashMap<K, u64>,
    after: &HashMap<K, u64>,
) -> HashMap<K, Delta> {
    let mut result = HashMap::new();
    for (key, &size) in before {
        result.insert(key.clone(), Delta {
            before: Some(size),
            after: after.get(key).copied(),
        });
    }
    for (key, &size) in after {
        result.entry(key.clone()).or_insert(Delta {
            before: None,
            after: Some(size),
        });
    }
    result
}

/// Sum before/after totals across every entry in `deltas` that belongs to
/// `group` — i.e. every symbol logically part of a hovered directory or
/// cluster, in *either* file.
///
/// Membership is a predicate over the symbol's identity (`Group::contains`),
/// not a list collected from whichever tree was hovered: a symbol only added
/// in the "after" file has no leaf in the "before" tree, and a whole source
/// file added or removed adds sources the hovered side never saw, so any set
/// derived from one side's own leaves silently drops exactly the additions and
/// removals that matter.
pub fn aggregate_group(deltas: &HashMap<SymbolKey, Delta>, group: &Group) -> (u64, u64) {
    let mut total_before = 0u64;
    let mut total_after = 0u64;
    for (key, delta) in deltas {
        if group.contains(key) {
            total_before += delta.before.unwrap_or(0);
            total_after += delta.after.unwrap_or(0);
        }
    }
    (total_before, total_after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unchanged_symbol() {
        let before: HashMap<String, u64> = HashMap::from([("a".into(), 100u64)]);
        let after: HashMap<String, u64> = HashMap::from([("a".into(), 100u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("a").unwrap();
        assert_eq!(d.diff_bytes(), 0);
    }

    #[test]
    fn test_grew_symbol() {
        let before: HashMap<String, u64> = HashMap::from([("a".into(), 100u64)]);
        let after: HashMap<String, u64> = HashMap::from([("a".into(), 150u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("a").unwrap();
        assert_eq!(d.diff_bytes(), 50);
    }

    #[test]
    fn test_new_symbol() {
        let before: HashMap<String, u64> = HashMap::new();
        let after: HashMap<String, u64> = HashMap::from([("new_sym".into(), 200u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("new_sym").unwrap();
        assert_eq!(d.before, None);
        assert_eq!(d.after, Some(200));
    }

    #[test]
    fn test_removed_symbol() {
        let before: HashMap<String, u64> = HashMap::from([("old_sym".into(), 300u64)]);
        let after: HashMap<String, u64> = HashMap::new();
        let diff = compute_diff(&before, &after);
        let d = diff.get("old_sym").unwrap();
        assert_eq!(d.before, Some(300));
        assert_eq!(d.after, None);
    }

    #[test]
    fn test_diff_pct_positive() {
        let d = Delta { before: Some(100), after: Some(150) };
        assert!((d.diff_pct() - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_diff_pct_new_is_infinity() {
        let d = Delta { before: None, after: Some(100) };
        assert!(d.diff_pct().is_infinite());
    }

    #[test]
    fn test_diff_pct_removed_is_neg_infinity() {
        // Regression: a removed symbol (before=Some, after=None) must report
        // NEG_INFINITY, not -100.0 — otherwise delta_color() renders it as
        // "shrank 100%" (green) instead of "removed" (bright red). Reported by
        // Chris: https://github.com/bondhome/elfvis/pull/2#issuecomment (color
        // direction "seems to be backwards").
        let d = Delta { before: Some(100), after: None };
        assert_eq!(d.diff_pct(), f64::NEG_INFINITY);
    }

    #[test]
    fn test_compute_diff_disambiguates_same_named_symbols_by_key() {
        // Regression for the code-review finding: `helper` defined in both
        // a.c and b.c must not collapse into a single delta. Keying by
        // `SymbolKey { source, name }` (instead of bare name) keeps them
        // distinct even though `name` alone collides.
        use crate::tree::SymbolKey;

        let a_helper = SymbolKey { source: Some("a.c".into()), name: "helper".into(), tu: None };
        let b_helper = SymbolKey { source: Some("b.c".into()), name: "helper".into(), tu: None };

        let before = HashMap::from([(a_helper.clone(), 22u64), (b_helper.clone(), 22u64)]);
        let after = HashMap::from([(a_helper.clone(), 50u64), (b_helper.clone(), 22u64)]);

        let diff = compute_diff(&before, &after);
        assert_eq!(diff.get(&a_helper).unwrap().diff_bytes(), 28, "a.c::helper grew 22 -> 50");
        assert_eq!(diff.get(&b_helper).unwrap().diff_bytes(), 0, "b.c::helper is unchanged");
    }

    #[test]
    fn test_removed_symbol_colors_bright_red_not_green() {
        use crate::color::delta_color;

        let d = Delta { before: Some(100), after: None };
        let c = delta_color(d.diff_pct());
        assert!(
            c.r > 150 && c.g < 150,
            "removed symbol should render bright red like the new-symbol case, \
             got rgb({},{},{})",
            c.r,
            c.g,
            c.b
        );
    }
}
