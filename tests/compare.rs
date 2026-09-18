//! End-to-end comparison tests: real ARM ELFs -> parser -> `compare`. These
//! exist because hand-built keys/paths assume the very things (stable source
//! paths, distinguishable duplicates) the parser has to actually deliver.
//! Fixtures are built by `tests/fixtures/build_compare.sh`.

use elfvis::compare::{compare, Comparison};
use elfvis::diff::aggregate_group;
use elfvis::parse::{parse_elf_detailed, SymbolDetail};
use elfvis::layout::{hit_test, layout, LayoutNode};
use elfvis::tree::{Group, SizeNode};

fn load(bytes: &[u8]) -> Vec<SymbolDetail> {
    parse_elf_detailed(bytes).unwrap()
}

fn find<'a>(node: &'a SizeNode, name: &str) -> Option<&'a SizeNode> {
    if node.name == name {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, name))
}

fn leaves<'a>(node: &'a SizeNode, out: &mut Vec<&'a SizeNode>) {
    if node.children.is_empty() {
        out.push(node);
    }
    for c in &node.children {
        leaves(c, out);
    }
}

fn delta_named<'a>(cmp: &'a Comparison, name: &str) -> Vec<&'a elfvis::diff::Delta> {
    cmp.deltas.iter().filter(|(k, _)| k.name == name).map(|(_, d)| d).collect()
}

#[test]
fn unchanged_symbol_keeps_one_identity_when_the_per_elf_path_prefix_changes() {
    // sub/a.c + sub/b.c has common prefix ".../sub/"; adding c.c beside sub/
    // moves it to ".../". The per-ELF display path of `fa` therefore changes
    // ("a.c" -> "sub/a.c") even though the function is byte-identical.
    let a = load(include_bytes!("fixtures/prefix_before.elf"));
    let b = load(include_bytes!("fixtures/prefix_after.elf"));

    let fa_a = a.iter().find(|s| s.name == "fa").unwrap();
    let fa_b = b.iter().find(|s| s.name == "fa").unwrap();
    assert_ne!(fa_a.display_path, fa_b.display_path, "precondition: display paths really differ");

    let cmp = compare(&a, &b);
    let fa = delta_named(&cmp, "fa");
    assert_eq!(fa.len(), 1, "unchanged fa must be a single delta, not removed+added: {fa:?}");
    assert_eq!((fa[0].before, fa[0].after), (Some(22), Some(22)));
    assert_eq!(delta_named(&cmp, "fc").len(), 1);
    assert_eq!(delta_named(&cmp, "fc")[0].before, None);

    // The same identity is what both trees' leaves carry, so highlighting works.
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    leaves(&cmp.tree_a, &mut la);
    leaves(&cmp.tree_b, &mut lb);
    let key_a = la.iter().find(|n| n.name == "fa").unwrap().key.clone();
    let key_b = lb.iter().find(|n| n.name == "fa").unwrap().key.clone();
    assert_eq!(key_a, key_b);
}

#[test]
fn same_named_statics_in_one_shared_header_stay_distinct() {
    // `static helper()` lives in shared.h, included by a.c and b.c: two local
    // symbols, both attributed to shared.h. Only a.c's copy grows.
    let a = load(include_bytes!("fixtures/shared_header_before.elf"));
    let b = load(include_bytes!("fixtures/shared_header_after.elf"));

    let helpers: Vec<_> = a.iter().filter(|s| s.name == "helper").collect();
    assert_eq!(helpers.len(), 2, "precondition: two helper symbols");
    assert_eq!(helpers[0].raw_path, helpers[1].raw_path, "precondition: same attributed source");
    assert_ne!(helpers[0].tu, helpers[1].tu, "the ELF's STT_FILE provenance tells them apart");

    let cmp = compare(&a, &b);
    assert!(cmp.ambiguous.is_empty());
    let mut diffs: Vec<i64> = delta_named(&cmp, "helper").iter().map(|d| d.diff_bytes()).collect();
    diffs.sort();
    assert_eq!(diffs.len(), 2, "one delta per helper, not one overwritten entry");
    assert_eq!(diffs[0], 0, "b.c's copy is unchanged");
    assert!(diffs[1] > 0, "a.c's copy grew");
}

#[test]
fn directory_total_is_the_same_from_both_sides_when_a_whole_file_is_added() {
    // src/ holds a.c + b.c; the second ELF adds src/c.c. The hovered `src`
    // node exists in both trees but only the new one has c.c's symbol.
    let a = load(include_bytes!("fixtures/group_before.elf"));
    let b = load(include_bytes!("fixtures/group_after.elf"));
    let cmp = compare(&a, &b);

    let group_a = find(&cmp.tree_a, "src").and_then(|n| n.group.clone()).expect("src in old tree");
    let group_b = find(&cmp.tree_b, "src").and_then(|n| n.group.clone()).expect("src in new tree");
    assert_eq!(group_a, group_b, "same logical group regardless of which tree is hovered");
    assert_eq!(group_a, Group::Dir(vec!["src".into()]));

    let added = delta_named(&cmp, "added")[0].after.unwrap();
    let (before, after) = aggregate_group(&cmp.deltas, &group_a);
    assert_eq!(before, 44, "keep + fixed");
    assert_eq!(after, 44 + added, "keep + fixed + the added file's symbol");
}

#[test]
fn files_at_the_build_root_and_in_subdirectories_align_together() {
    // m.c sits in the compilation unit's comp_dir (absolute in DWARF) while
    // src/*.c are recorded relative to it. Built in two different directories,
    // every file must still line up — `_start` in m.c is unchanged.
    let a = load(include_bytes!("fixtures/group_before.elf"));
    let b = load(include_bytes!("fixtures/group_after.elf"));

    assert!(
        a.iter().filter_map(|s| s.raw_path.as_deref()).all(|p| p.starts_with('/')),
        "raw paths are resolved against comp_dir, not a mix of relative and absolute"
    );

    let cmp = compare(&a, &b);
    let start = delta_named(&cmp, "_start");
    assert_eq!(start.len(), 1, "unchanged _start must not become removed+added: {start:?}");
    assert_eq!(start[0].diff_bytes(), 0);
    assert!(cmp.deltas.values().filter(|d| d.before.is_none()).count() == 1, "only `added` is new");
    assert!(cmp.deltas.values().all(|d| d.after.is_some()), "nothing was removed");
}

#[test]
fn hovering_a_same_named_sibling_selects_that_symbol_not_the_first() {
    // Both `helper`s sit under shared.h. Hit-test the centre of the unchanged
    // b.c copy in the new tree; the node under the cursor (and therefore the
    // tooltip delta and the cross-highlight) must be b.c's, not a.c's.
    let a = load(include_bytes!("fixtures/shared_header_before.elf"));
    let b = load(include_bytes!("fixtures/shared_header_after.elf"));
    let cmp = compare(&a, &b);
    let root = layout(&cmp.tree_b, 800.0, 600.0);

    fn helpers<'a>(n: &'a LayoutNode, out: &mut Vec<&'a LayoutNode>) {
        if n.is_leaf && n.name == "helper" {
            out.push(n);
        }
        for c in &n.children {
            helpers(c, out);
        }
    }
    let mut hs = Vec::new();
    helpers(&root, &mut hs);
    assert_eq!(hs.len(), 2);
    let from_b = hs.iter().find(|n| n.key.as_ref().unwrap().tu.as_deref() == Some("b.c")).unwrap();

    let chain = hit_test(&root, from_b.rect.x + from_b.rect.w / 2.0, from_b.rect.y + from_b.rect.h / 2.0).unwrap();
    let hit = chain.last().unwrap();
    assert!(std::ptr::eq(*hit, *from_b), "hit node must be b.c's helper");

    let delta = &cmp.deltas[hit.key.as_ref().unwrap()];
    assert_eq!(delta.diff_bytes(), 0, "b.c's helper is unchanged; a.c's +N must not leak in");
}

#[test]
fn different_files_with_the_same_basename_are_not_merged_by_root_alignment() {
    // Before: src/a.c + old/b.c. After: the same src/a.c + a different new/b.c,
    // each with its own local `helper`. src/a.c is an exact anchor at the shared
    // root, so the alignment must not strip `old/`/`new/` just to make the two
    // b.c basenames coincide.
    let a = load(include_bytes!("fixtures/dir_move_before.elf"));
    let b = load(include_bytes!("fixtures/dir_move_after.elf"));
    let cmp = compare(&a, &b);

    let helpers = delta_named(&cmp, "helper");
    assert_eq!(helpers.len(), 2, "old/b.c and new/b.c helpers are separate identities: {helpers:?}");
    assert!(
        helpers.iter().all(|d| d.before.is_none() || d.after.is_none()),
        "one removed, one added — not a single 22 -> N change: {helpers:?}"
    );
    let use_a = delta_named(&cmp, "use_a");
    assert_eq!(use_a.len(), 1);
    assert_eq!((use_a[0].before, use_a[0].after), (Some(22), Some(22)), "the anchor file still matches");
}

#[test]
fn local_translation_unit_identity_is_kept_when_each_side_has_one_copy() {
    // Exactly one static helper() per ELF, but from different translation units
    // (a.c vs b.c) that include the same header, so the raw source is identical.
    let a = load(include_bytes!("fixtures/tu_before.elf"));
    let b = load(include_bytes!("fixtures/tu_after.elf"));
    let ha = a.iter().find(|s| s.name == "helper").unwrap();
    let hb = b.iter().find(|s| s.name == "helper").unwrap();
    assert_eq!(ha.raw_path.as_deref().map(|p| p.rsplit('/').next()), hb.raw_path.as_deref().map(|p| p.rsplit('/').next()));
    assert_ne!(ha.tu, hb.tu, "precondition: provenance differs");

    let cmp = compare(&a, &b);
    let helpers = delta_named(&cmp, "helper");
    assert_eq!(helpers.len(), 2, "two different local definitions: {helpers:?}");
    assert!(helpers.iter().all(|d| d.before.is_none() || d.after.is_none()));
    assert!(cmp.ambiguous.is_empty(), "provenance is sufficient here; nothing is ambiguous");
}
