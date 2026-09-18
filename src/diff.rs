use std::collections::HashMap;

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

pub fn compute_diff(before: &HashMap<String, u64>, after: &HashMap<String, u64>) -> HashMap<String, Delta> {
    let mut result = HashMap::new();
    for (path, &size) in before {
        result.insert(path.clone(), Delta {
            before: Some(size),
            after: after.get(path).copied(),
        });
    }
    for (path, &size) in after {
        result.entry(path.clone()).or_insert(Delta {
            before: None,
            after: Some(size),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unchanged_symbol() {
        let before = HashMap::from([("a".into(), 100u64)]);
        let after = HashMap::from([("a".into(), 100u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("a").unwrap();
        assert_eq!(d.diff_bytes(), 0);
    }

    #[test]
    fn test_grew_symbol() {
        let before = HashMap::from([("a".into(), 100u64)]);
        let after = HashMap::from([("a".into(), 150u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("a").unwrap();
        assert_eq!(d.diff_bytes(), 50);
    }

    #[test]
    fn test_new_symbol() {
        let before = HashMap::new();
        let after = HashMap::from([("new_sym".into(), 200u64)]);
        let diff = compute_diff(&before, &after);
        let d = diff.get("new_sym").unwrap();
        assert_eq!(d.before, None);
        assert_eq!(d.after, Some(200));
    }

    #[test]
    fn test_removed_symbol() {
        let before = HashMap::from([("old_sym".into(), 300u64)]);
        let after = HashMap::new();
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
