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
//! 2. **Identity includes translation unit for locals.** A local symbol's name
//!    is only unique within its object file, and the source it is attributed to
//!    can be shared (a `static` in a header included by several `.c` files), so
//!    `(source, name, translation unit)` is the identity — always, not only when
//!    a duplicate happens to be present on one side: with one copy per ELF, a
//!    `helper` from `a.c` and a `helper` from `b.c` are still different symbols.
//!    The translation unit comes from the ELF's `STT_FILE` provenance (reduced
//!    to its basename, so it does not depend on how the compiler was invoked).
//! 3. **Explicit ambiguity.** If an identity still collides (no provenance, or
//!    equal `STT_FILE` names), unique matching cannot be established. Those keys
//!    are reported in [`Comparison::ambiguous`] (with combined sizes) instead of
//!    one duplicate silently overwriting another in a map.
//! 4. **Joint clustering.** Which unresolved-symbol name prefixes form named
//!    clusters (versus the `<other>` catch-all) is decided once for the pair, so
//!    a group means the same thing in both trees.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::diff::{compute_diff, Delta};
use crate::parse::{common_dir_prefix_len, ResolvedSymbol, SymbolDetail};
use crate::tree::{build_tree_clustered, unresolved_cluster_prefixes, SizeNode, SymbolKey};

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

/// Number of leading path components shared by *every* file in `files` — the
/// deepest directory all of a side's sources live under.
fn shared_depth(files: &BTreeSet<&str>) -> usize {
    let len = common_dir_prefix_len(files.iter().copied());
    files.iter().next().map_or(0, |f| components(&f[..len]).len())
}

/// How many leading path components to drop from each side so that the same
/// source file gets the same path in both ELFs.
///
/// The two builds may live under different roots (`/home/a/proj/src/x.c` vs
/// `/ci/ws/src/x.c`), so this picks the pair of strip counts that makes the
/// most source files coincide; ties prefer stripping less. It only ever removes
/// a side's *shared* leading directories (never more than [`shared_depth`]):
/// components that distinguish one of its files from another (`old/` vs `src/`)
/// are real project structure, and stripping them just to make basenames line up
/// would invent matches between different files. It looks only at the files,
/// never at which symbols they contain, so it does not shift when a file is added.
fn align_roots(a: &BTreeSet<&str>, b: &BTreeSet<&str>) -> (usize, usize) {
    let stripped = |files: &BTreeSet<&str>| -> Vec<HashSet<String>> {
        let comps: Vec<Vec<&str>> = files.iter().map(|f| components(f)).collect();
        (0..=shared_depth(files))
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

    let side_a = build_side(a, sources_a);
    let side_b = build_side(b, sources_b);

    let ambiguous: HashSet<SymbolKey> = side_a
        .counts
        .iter()
        .chain(side_b.counts.iter())
        .filter(|(_, &n)| n > 1)
        .map(|(k, _)| k.clone())
        .collect();

    let mut clustered = unresolved_cluster_prefixes(&side_a.symbols);
    clustered.extend(unresolved_cluster_prefixes(&side_b.symbols));

    Comparison {
        deltas: compute_diff(&side_a.sizes, &side_b.sizes),
        tree_a: build_tree_clustered(&side_a.symbols, &side_a.keys, &clustered),
        tree_b: build_tree_clustered(&side_b.symbols, &side_b.keys, &clustered),
        ambiguous,
    }
}

/// Translation-unit part of a local symbol's identity: the `STT_FILE` name's
/// basename, so `sub/a.c` and `./a.c` (same object compiled with different
/// invocations) agree.
fn tu_identity(tu: Option<&str>) -> Option<String> {
    tu.map(|t| t.rsplit('/').next().unwrap_or(t).to_string())
}

fn build_side(details: &[SymbolDetail], sources: Vec<Option<String>>) -> Side {
    let mut side = Side {
        symbols: Vec::with_capacity(details.len()),
        keys: Vec::with_capacity(details.len()),
        sizes: HashMap::new(),
        counts: HashMap::new(),
    };
    for (d, source) in details.iter().zip(sources) {
        let key = SymbolKey { source: source.clone(), name: d.name.clone(), tu: tu_identity(d.tu.as_deref()) };
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

    #[test]
    fn test_other_catch_all_means_the_same_in_both_trees() {
        // `<other>` used to carry a prefix list taken from each tree's own
        // singletons ([spare] vs [added, spare]), so hovering it gave 10->10 on
        // one side and 10->30 on the other. Which prefixes are named clusters is
        // now decided jointly, and `<other>` is "unresolved and not clustered".
        let a = [sym("motor_init", 100, None, None), sym("motor_step", 50, None, None), sym("spare_one", 10, None, None)];
        let b = [
            sym("motor_init", 100, None, None),
            sym("motor_step", 50, None, None),
            sym("spare_one", 10, None, None),
            sym("added_one", 20, None, None),
        ];
        let cmp = compare(&a, &b);
        for tree in [&cmp.tree_a, &cmp.tree_b] {
            let group = find(tree, "<other>").and_then(|n| n.group.clone()).expect("<other>");
            assert_eq!(crate::diff::aggregate_group(&cmp.deltas, &group), (10, 30));
        }
    }

    #[test]
    fn test_prefix_that_becomes_a_cluster_on_one_side_is_a_cluster_on_both() {
        // `added_*` is a singleton in the old file but a pair in the new one.
        // It must be a named cluster in both trees (not <other> in one).
        let a = [sym("added_one", 20, None, None), sym("spare_x", 10, None, None)];
        let b = [sym("added_one", 20, None, None), sym("added_two", 30, None, None), sym("spare_x", 10, None, None)];
        let cmp = compare(&a, &b);
        for tree in [&cmp.tree_a, &cmp.tree_b] {
            let group = find(tree, "added").and_then(|n| n.group.clone()).expect("added cluster in both trees");
            assert_eq!(crate::diff::aggregate_group(&cmp.deltas, &group), (20, 50));
        }
    }

    #[test]
    fn test_alignment_never_strips_directories_that_distinguish_files() {
        // Shared root /w; old/b.c vs new/b.c are different files even though
        // the basenames agree. Only /w/src/a.c is a real anchor.
        let a = [sym("use_a", 22, Some("/w/src/a.c"), None), sym("helper", 22, Some("/w/old/b.c"), Some("b.c"))];
        let b = [sym("use_a", 22, Some("/w/src/a.c"), None), sym("helper", 50, Some("/w/new/b.c"), Some("b.c"))];
        let cmp = compare(&a, &b);
        let helpers: Vec<_> = cmp.deltas.iter().filter(|(k, _)| k.name == "helper").collect();
        assert_eq!(helpers.len(), 2, "{helpers:?}");
    }
}
