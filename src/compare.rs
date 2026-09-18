//! Comparison of two parsed ELFs: symbol identity, diffing, and the two display
//! trees, as one pure pipeline (no DOM) so it can be tested end to end.
//!
//! The steps are order-dependent and each one exists to keep two independently
//! parsed files comparable:
//!
//! 1. **Pair-level path normalization.** Each ELF's own display paths have a
//!    prefix stripped that depends on which files *that* ELF contains, so the
//!    same source file can be `a.c` in one and `sub/a.c` in the other. Identity
//!    is therefore derived from the raw DWARF paths, normalized against each
//!    other: first the two build roots are aligned (they may differ, e.g. two
//!    checkouts), then one shared prefix is trimmed from both.
//! 2. **Joint key assignment.** `(source, name)` identifies a symbol unless it
//!    collides (same-named locals in different object files, even attributed to
//!    one shared header). Wherever it collides in *either* file, both files
//!    refine that key with the translation unit from the ELF's `STT_FILE`
//!    provenance, so the refinement is applied consistently to the pair.
//! 3. **Explicit ambiguity.** If a key still collides after refinement, unique
//!    matching cannot be established. Those keys are reported in
//!    [`Comparison::ambiguous`] (with combined sizes) instead of one duplicate
//!    silently overwriting another in a map.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::diff::{compute_diff, Delta};
use crate::parse::{common_dir_prefix_len, ResolvedSymbol, SymbolDetail};
use crate::tree::{build_tree_with_keys, SizeNode, SymbolKey};

pub struct Comparison {
    pub tree_a: SizeNode,
    pub tree_b: SizeNode,
    pub deltas: HashMap<SymbolKey, Delta>,
    /// Keys shared by more than one symbol in at least one file. Their deltas
    /// are combined totals and must not be presented as a per-symbol change.
    pub ambiguous: HashSet<SymbolKey>,
}

struct Side {
    symbols: Vec<ResolvedSymbol>,
    keys: Vec<SymbolKey>,
    sizes: HashMap<SymbolKey, u64>,
    counts: HashMap<SymbolKey, u32>,
}

fn components(path: &str) -> Vec<&str> {
    path.split('/').filter(|c| !c.is_empty()).collect()
}

/// How many leading path components to drop from each side so that the same
/// source file gets the same path in both ELFs.
///
/// The two builds may live under different roots (`/home/a/proj/src/x.c` vs
/// `/ci/ws/src/x.c`), so this picks the pair of strip counts that makes the
/// most source files coincide; ties prefer stripping less, which keeps as much
/// real path information as possible. It looks only at the files, never at
/// which symbols they contain, so it does not shift when a file is added.
fn align_roots(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> (usize, usize) {
    let stripped = |files: &BTreeSet<&str>| -> Vec<HashSet<String>> {
        let comps: Vec<Vec<&str>> = files.iter().map(|f| components(f)).collect();
        let max_strip = comps.iter().map(|c| c.len()).max().unwrap_or(1);
        (0..max_strip)
            .map(|n| comps.iter().filter(|c| c.len() > n).map(|c| c[n..].join("/")).collect())
            .collect()
    };
    let (sa, sb) = (stripped(a), stripped(b));

    let mut best = (0usize, 0usize);
    let mut best_score = 0usize;
    for (ca, set_a) in sa.iter().enumerate() {
        for (cb, set_b) in sb.iter().enumerate() {
            let score = set_a.intersection(set_b).count();
            let better = score > best_score
                || (score == best_score && score > 0 && (ca + cb, ca) < (best.0 + best.1, best.0));
            if better {
                best = (ca, cb);
                best_score = score;
            }
        }
    }
    best
}

/// Source path of each symbol, aligned across the pair and with their shared
/// leading directories trimmed for readability.
fn normalize_sources(a: &[SymbolDetail], b: &[SymbolDetail]) -> (Vec<Option<String>>, Vec<Option<String>>) {
    fn files(syms: &[SymbolDetail]) -> BTreeSet<&str> {
        syms.iter().filter_map(|s| s.raw_path.as_deref()).collect()
    }
    let (ca, cb) = align_roots(&files(a), &files(b));

    let aligned = |syms: &[SymbolDetail], strip: usize| -> Vec<Option<String>> {
        syms.iter()
            .map(|s| {
                s.raw_path.as_deref().map(|p| {
                    let c = components(p);
                    c[strip.min(c.len().saturating_sub(1))..].join("/")
                })
            })
            .collect()
    };
    let (mut na, mut nb) = (aligned(a, ca), aligned(b, cb));

    let prefix = common_dir_prefix_len(na.iter().chain(nb.iter()).filter_map(|p| p.as_deref()));
    for p in na.iter_mut().chain(nb.iter_mut()).flatten() {
        p.drain(..prefix);
    }
    (na, nb)
}

pub fn compare(a: &[SymbolDetail], b: &[SymbolDetail]) -> Comparison {
    let (sources_a, sources_b) = normalize_sources(a, b);

    let mut collided: HashSet<(Option<String>, String)> = HashSet::new();
    for (syms, sources) in [(a, &sources_a), (b, &sources_b)] {
        let mut seen: HashSet<(&Option<String>, &str)> = HashSet::new();
        for (sym, source) in syms.iter().zip(sources) {
            if !seen.insert((source, sym.name.as_str())) {
                collided.insert((source.clone(), sym.name.clone()));
            }
        }
    }

    let side_a = build_side(a, sources_a, &collided);
    let side_b = build_side(b, sources_b, &collided);

    let ambiguous: HashSet<SymbolKey> = side_a
        .counts
        .iter()
        .chain(side_b.counts.iter())
        .filter(|(_, &n)| n > 1)
        .map(|(k, _)| k.clone())
        .collect();

    Comparison {
        deltas: compute_diff(&side_a.sizes, &side_b.sizes),
        tree_a: build_tree_with_keys(&side_a.symbols, &side_a.keys),
        tree_b: build_tree_with_keys(&side_b.symbols, &side_b.keys),
        ambiguous,
    }
}

fn build_side(
    details: &[SymbolDetail],
    sources: Vec<Option<String>>,
    collided: &HashSet<(Option<String>, String)>,
) -> Side {
    let mut side = Side {
        symbols: Vec::with_capacity(details.len()),
        keys: Vec::with_capacity(details.len()),
        sizes: HashMap::new(),
        counts: HashMap::new(),
    };
    for (d, source) in details.iter().zip(sources) {
        let tu = if collided.contains(&(source.clone(), d.name.clone())) { d.tu.clone() } else { None };
        let key = SymbolKey { source: source.clone(), name: d.name.clone(), tu };
        *side.sizes.entry(key.clone()).or_insert(0) += d.size;
        *side.counts.entry(key.clone()).or_insert(0) += 1;
        side.symbols.push(ResolvedSymbol { name: d.name.clone(), size: d.size, source_path: source });
        side.keys.push(key);
    }
    side
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, size: u64, raw: Option<&str>, tu: Option<&str>) -> SymbolDetail {
        SymbolDetail {
            name: name.into(),
            size,
            raw_path: raw.map(str::to_string),
            display_path: None,
            tu: tu.map(str::to_string),
        }
    }

    #[test]
    fn test_prefix_is_normalized_over_the_pair_not_per_file() {
        // Old side only has files under /w/sub/, so its own common prefix would
        // be "/w/sub/"; the new side adds /w/c.c, making its prefix "/w/". The
        // shared prefix is "/w/" for both, so `fa` keeps one identity.
        let a = [sym("fa", 22, Some("/w/sub/a.c"), None), sym("fb", 22, Some("/w/sub/b.c"), None)];
        let b = [
            sym("fa", 22, Some("/w/sub/a.c"), None),
            sym("fb", 22, Some("/w/sub/b.c"), None),
            sym("fc", 22, Some("/w/c.c"), None),
        ];
        let cmp = compare(&a, &b);
        let fa: Vec<_> = cmp.deltas.iter().filter(|(k, _)| k.name == "fa").collect();
        assert_eq!(fa.len(), 1, "unchanged fa must be a single delta: {fa:?}");
        assert_eq!(fa[0].1.diff_bytes(), 0);
        assert!(cmp.ambiguous.is_empty());
    }

    #[test]
    fn test_same_source_same_name_locals_are_split_by_translation_unit() {
        let a = [
            sym("helper", 22, Some("/w/shared.h"), Some("a.c")),
            sym("helper", 22, Some("/w/shared.h"), Some("b.c")),
        ];
        let b = [
            sym("helper", 50, Some("/w/shared.h"), Some("a.c")),
            sym("helper", 22, Some("/w/shared.h"), Some("b.c")),
        ];
        let cmp = compare(&a, &b);
        assert!(cmp.ambiguous.is_empty(), "TU provenance makes them distinguishable");
        let mut diffs: Vec<i64> = cmp.deltas.values().map(|d| d.diff_bytes()).collect();
        diffs.sort();
        assert_eq!(diffs, vec![0, 28]);
    }

    #[test]
    fn test_unresolved_same_name_locals_are_split_by_translation_unit() {
        let a = [sym("helper", 10, None, Some("a.c")), sym("helper", 10, None, Some("b.c"))];
        let b = [sym("helper", 30, None, Some("a.c")), sym("helper", 10, None, Some("b.c"))];
        let cmp = compare(&a, &b);
        let mut diffs: Vec<i64> = cmp.deltas.values().map(|d| d.diff_bytes()).collect();
        diffs.sort();
        assert_eq!(diffs, vec![0, 20]);
    }

    #[test]
    fn test_collision_is_refined_in_both_files_even_if_only_one_collides() {
        // Old has a single `helper`; new gained a second one in another TU.
        // The key must be refined identically on both sides or the unchanged
        // one would look removed+added.
        let a = [sym("helper", 22, Some("/w/shared.h"), Some("a.c"))];
        let b = [
            sym("helper", 22, Some("/w/shared.h"), Some("a.c")),
            sym("helper", 22, Some("/w/shared.h"), Some("b.c")),
        ];
        let cmp = compare(&a, &b);
        let unchanged = cmp.deltas.values().filter(|d| d.before.is_some() && d.after.is_some()).count();
        let added = cmp.deltas.values().filter(|d| d.before.is_none()).count();
        assert_eq!((unchanged, added), (1, 1));
    }

    #[test]
    fn test_unresolvable_collision_is_reported_ambiguous_not_overwritten() {
        // No translation-unit provenance at all: cannot tell them apart.
        let a = [sym("helper", 22, Some("/w/shared.h"), None), sym("helper", 22, Some("/w/shared.h"), None)];
        let b = [sym("helper", 50, Some("/w/shared.h"), None), sym("helper", 22, Some("/w/shared.h"), None)];
        let cmp = compare(&a, &b);
        assert_eq!(cmp.ambiguous.len(), 1);
        let d = cmp.deltas.values().next().unwrap();
        assert_eq!((d.before, d.after), (Some(44), Some(72)), "combined totals, not last-writer-wins");
    }

    #[test]
    fn test_builds_in_different_roots_still_match() {
        // Same project checked out at two different depths (a developer's home
        // dir vs. a CI workspace). The same relative files must line up, and
        // adding a file must not break that.
        let a = [
            sym("fa", 22, Some("/home/u/proj/src/a.c"), None),
            sym("fb", 22, Some("/home/u/proj/src/b.c"), None),
        ];
        let b = [
            sym("fa", 22, Some("/ci/ws/src/a.c"), None),
            sym("fb", 30, Some("/ci/ws/src/b.c"), None),
            sym("fc", 22, Some("/ci/ws/lib/c.c"), None),
        ];
        let cmp = compare(&a, &b);
        let pairs: Vec<_> = cmp.deltas.values().filter(|d| d.before.is_some() && d.after.is_some()).collect();
        assert_eq!(pairs.len(), 2, "fa and fb matched across roots: {:?}", cmp.deltas);
        assert_eq!(cmp.deltas.values().filter(|d| d.before.is_none()).count(), 1, "only fc is new");
    }

    fn find<'a>(node: &'a SizeNode, name: &str) -> Option<&'a SizeNode> {
        if node.name == name {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, name))
    }

    #[test]
    fn test_unresolved_cluster_total_includes_a_member_only_the_other_side_has() {
        // motor_init + motor_step cluster under "motor"; the new file adds
        // motor_extra (same name prefix). Hovering the old "motor" cluster must
        // total 150 -> 230, not 150 -> 150.
        let a = [
            sym("motor_init", 100, None, None),
            sym("motor_step", 50, None, None),
            sym("other_x", 10, None, None),
            sym("other_y", 10, None, None),
        ];
        let b = [
            sym("motor_init", 100, None, None),
            sym("motor_step", 50, None, None),
            sym("motor_extra", 80, None, None),
            sym("other_x", 10, None, None),
            sym("other_y", 10, None, None),
        ];
        let cmp = compare(&a, &b);
        for tree in [&cmp.tree_a, &cmp.tree_b] {
            let group = find(tree, "motor").and_then(|n| n.group.clone()).expect("motor cluster");
            assert_eq!(crate::diff::aggregate_group(&cmp.deltas, &group), (150, 230));
        }
    }

    #[test]
    fn test_removed_whole_file_is_counted_when_hovering_the_side_that_lost_it() {
        let a = [
            sym("keep", 100, Some("/w/src/a.c"), None),
            sym("fixed", 50, Some("/w/src/b.c"), None),
            sym("gone", 80, Some("/w/src/c.c"), None),
            sym("lib_fn", 5, Some("/w/lib/x.c"), None),
        ];
        let b = [
            sym("keep", 100, Some("/w/src/a.c"), None),
            sym("fixed", 50, Some("/w/src/b.c"), None),
            sym("lib_fn", 5, Some("/w/lib/x.c"), None),
        ];
        let cmp = compare(&a, &b);
        for tree in [&cmp.tree_a, &cmp.tree_b] {
            let group = find(tree, "src").and_then(|n| n.group.clone()).expect("src dir");
            assert_eq!(crate::diff::aggregate_group(&cmp.deltas, &group), (230, 150));
        }
    }
}
