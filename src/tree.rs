use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Stable identity for a symbol (leaf), independent of the display tree's
/// clustering/collapsing.
///
/// `source` is `None` when DWARF could not place the symbol. `tu` is the
/// translation unit of a local symbol (from the ELF's `STT_FILE` provenance),
/// `None` for globals and when the ELF carries no provenance. Two object files
/// may legitimately define same-named locals, even attributed to the same
/// header, so neither name nor source file is a translation-unit identity on
/// its own — see `compare::compare`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolKey {
    pub source: Option<String>,
    pub name: String,
    pub tu: Option<String>,
}

/// The default identity used by single-file mode: `(source, name)`.
pub fn symbol_key(sym: &crate::parse::ResolvedSymbol) -> SymbolKey {
    SymbolKey { source: sym.source_path.clone(), name: sym.name.clone(), tu: None }
}

/// A logical group of symbols that a directory-like tree node stands for,
/// defined as a predicate over `SymbolKey` rather than as "whatever leaves this
/// one tree happens to contain". Comparison mode needs that: the two trees are
/// built from different symbol sets, so a group's membership must be evaluated
/// over both inputs (e.g. a file added only in the new ELF still belongs to
/// its directory when the old ELF's directory node is hovered).
///
/// Every variant must mean the same thing in both trees of a comparison, so
/// none of them may embed a list derived from one tree's own contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    /// The whole tree.
    All,
    /// Symbols whose source path lies under this directory (path components).
    Dir(Vec<String>),
    /// Every symbol DWARF could not place.
    Unresolved,
    /// Unresolved symbols sharing this name-cluster prefix (see `extract_prefix`).
    UnresolvedCluster(String),
    /// Unresolved symbols whose prefix is *not* a named cluster — the
    /// `<other>` catch-all. The set of named clusters is fixed for the whole
    /// tree (or, in comparison mode, jointly for both trees).
    UnresolvedOther(Rc<HashSet<String>>),
}

impl Group {
    pub fn contains(&self, key: &SymbolKey) -> bool {
        match self {
            Group::All => true,
            Group::Dir(dir) => key.source.as_deref().is_some_and(|s| {
                let mut comps = s.split('/').filter(|c| !c.is_empty());
                dir.iter().all(|d| comps.next() == Some(d.as_str()))
            }),
            Group::Unresolved => key.source.is_none(),
            Group::UnresolvedCluster(prefix) => {
                key.source.is_none() && extract_prefix(&key.name) == *prefix
            }
            Group::UnresolvedOther(clustered) => {
                key.source.is_none() && !clustered.contains(&extract_prefix(&key.name))
            }
        }
    }
}

/// Name-cluster prefixes that have at least two unresolved symbols in `symbols`
/// — the rule `build_tree` uses to decide which prefixes get their own cluster
/// node (everything else goes to `<other>`).
pub fn unresolved_cluster_prefixes(symbols: &[crate::parse::ResolvedSymbol]) -> HashSet<String> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for s in symbols.iter().filter(|s| s.source_path.is_none()) {
        *counts.entry(extract_prefix(&s.name)).or_insert(0) += 1;
    }
    counts.into_iter().filter(|&(_, n)| n >= 2).map(|(p, _)| p).collect()
}

/// A node in the size tree.
#[derive(Debug, Clone, Default)]
pub struct SizeNode {
    /// Name of this node (directory name, filename, or symbol name).
    pub name: String,
    /// Total size of this node and all descendants (bytes).
    pub size: u64,
    /// Child nodes. Empty for leaf (symbol) nodes.
    pub children: Vec<SizeNode>,
    /// Stable identity, set only for leaf (symbol) nodes.
    pub key: Option<SymbolKey>,
    /// Logical membership of a directory-like node; `None` for leaves.
    pub group: Option<Group>,
}

/// Flatten a SizeNode tree into a map of full path -> leaf size.
/// Only leaf nodes (symbols) are included.
pub fn flatten_paths(tree: &SizeNode) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    flatten_recursive(tree, &mut String::new(), &mut map);
    map
}

fn flatten_recursive(node: &SizeNode, prefix: &mut String, map: &mut HashMap<String, u64>) {
    if node.children.is_empty() && !node.name.is_empty() {
        let key = if prefix.is_empty() {
            node.name.clone()
        } else {
            format!("{}/{}", prefix, node.name)
        };
        map.insert(key, node.size);
        return;
    }
    for child in &node.children {
        let old_len = prefix.len();
        if !node.name.is_empty() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(&node.name);
        }
        flatten_recursive(child, prefix, map);
        prefix.truncate(old_len);
    }
}

/// Build a size tree from resolved symbols.
/// Paths are split on '/' to create the directory hierarchy.
/// Symbols without a source path go under "<unknown>".
pub fn build_tree(symbols: &[crate::parse::ResolvedSymbol]) -> SizeNode {
    let keys: Vec<SymbolKey> = symbols.iter().map(symbol_key).collect();
    build_tree_with_keys(symbols, &keys)
}

/// Like [`build_tree`], but each leaf carries the caller-supplied identity in
/// `keys` (parallel to `symbols`) — comparison mode assigns those jointly over
/// both inputs so a symbol gets the same key in both trees.
pub fn build_tree_with_keys(symbols: &[crate::parse::ResolvedSymbol], keys: &[SymbolKey]) -> SizeNode {
    build_tree_clustered(symbols, keys, &unresolved_cluster_prefixes(symbols))
}

/// Like [`build_tree_with_keys`], with the set of unresolved-symbol prefixes
/// that get a named cluster given explicitly. Comparison mode decides that set
/// once for both trees so `<other>` means the same thing in each.
pub fn build_tree_clustered(
    symbols: &[crate::parse::ResolvedSymbol],
    keys: &[SymbolKey],
    clustered: &HashSet<String>,
) -> SizeNode {
    assert_eq!(symbols.len(), keys.len());
    let mut root = SizeNode {
        name: String::new(),
        size: 0,
        children: Vec::new(),
        key: None,
        group: Some(Group::All),
    };

    for (sym, key) in symbols.iter().zip(keys) {
        let path = match &sym.source_path {
            Some(p) => p.as_str(),
            None => "<unknown>",
        };

        // Split path into components, append symbol name as leaf
        let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        parts.push(&sym.name);

        // Walk/create the tree
        let mut node = &mut root;
        for (depth, part) in parts[..parts.len() - 1].iter().enumerate() {
            let idx = node.children.iter().position(|c| c.name == *part);
            let idx = match idx {
                Some(i) => i,
                None => {
                    let group = if sym.source_path.is_none() {
                        Group::Unresolved
                    } else {
                        Group::Dir(parts[..=depth].iter().map(|c| c.to_string()).collect())
                    };
                    node.children.push(SizeNode {
                        name: part.to_string(),
                        size: 0,
                        children: Vec::new(),
                        key: None,
                        group: Some(group),
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
        }

        // Add leaf symbol. `key` is a stable identity independent of this
        // display tree's clustering/collapsing.
        node.children.push(SizeNode {
            name: parts.last().unwrap().to_string(),
            size: sym.size,
            children: Vec::new(),
            key: Some(key.clone()),
            group: None,
        });
    }

    // Cluster unknown symbols by prefix
    if let Some(unknown) = root.children.iter_mut().find(|c| c.name == "<unknown>") {
        cluster_unknown_children(unknown, clustered);
    }

    // Compute sizes bottom-up and sort children by size descending
    compute_sizes(&mut root);
    for child in &mut root.children {
        collapse_single_children(child);
    }
    root
}

/// Collapse single-child directory chains.
/// When a directory has exactly one child that is also a directory,
/// merge them: `parent/child` absorbs grandchildren. Repeat until stable.
fn collapse_single_children(node: &mut SizeNode) {
    // First recurse into children
    for child in &mut node.children {
        collapse_single_children(child);
    }
    // Then collapse: while this node has exactly one child that is a directory
    while node.children.len() == 1 && !node.children[0].children.is_empty() {
        let only_child = node.children.remove(0);
        if node.name.is_empty() {
            node.name = only_child.name;
        } else {
            node.name = format!("{}/{}", node.name, only_child.name);
        }
        node.children = only_child.children;
        // A single-child chain has the same members top to bottom; keep the
        // deepest (most specific) group.
        node.group = only_child.group;
    }
}

fn compute_sizes(node: &mut SizeNode) {
    if node.children.is_empty() {
        return;
    }
    for child in &mut node.children {
        compute_sizes(child);
    }
    node.size = node.children.iter().map(|c| c.size).sum();
    node.children
        .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
}

/// Regroup flat children of the `<unknown>` node into prefix-based clusters.
/// Prefixes in `clustered` get a cluster node; the rest are merged into an
/// `<other>` catch-all.
fn cluster_unknown_children(node: &mut SizeNode, clustered: &HashSet<String>) {
    // Bucket children by prefix
    let mut buckets: HashMap<String, Vec<SizeNode>> = HashMap::new();
    for child in node.children.drain(..) {
        let prefix = extract_prefix(&child.name);
        buckets.entry(prefix).or_default().push(child);
    }

    let mut other_children: Vec<SizeNode> = Vec::new();
    for (prefix, children) in buckets {
        if clustered.contains(&prefix) {
            node.children.push(SizeNode {
                name: prefix.clone(),
                size: 0,
                children,
                key: None,
                group: Some(Group::UnresolvedCluster(prefix)),
            });
        } else {
            other_children.extend(children);
        }
    }

    // Add <other> if non-empty
    if !other_children.is_empty() {
        node.children.push(SizeNode {
            name: "<other>".to_string(),
            size: 0,
            children: other_children,
            key: None,
            group: Some(Group::UnresolvedOther(Rc::new(clustered.clone()))),
        });
    }
}

/// Extract a cluster prefix from a symbol name.
///
/// Rules (applied in order):
/// 1. `__*` → `"__"`
/// 2. `_*`  → `"_"`
/// 3. `gp_*` → strip `gp_`, extract token from remainder
/// 4. Single lowercase + `_` (Hungarian) → strip 2, extract token from remainder
/// 5. Single lowercase + uppercase (Hungarian) → strip 1, extract token from remainder
/// 6. Default → first token split on `_`, `.`, or camelCase boundary
pub fn extract_prefix(name: &str) -> String {
    if name.starts_with("__") {
        return "__".to_string();
    }
    if name.starts_with('_') {
        return "_".to_string();
    }

    // Hungarian notation: gp_ or single lowercase + (_ or uppercase)
    let chars: Vec<char> = name.chars().collect();
    let rest = if name.starts_with("gp_") {
        &name[3..]
    } else if chars.len() >= 2 && chars[0].is_lowercase() {
        if chars[1] == '_' {
            &name[2..]
        } else if chars[1].is_uppercase() {
            &name[1..]
        } else {
            name
        }
    } else {
        name
    };

    // Split on first underscore or dot
    if let Some(idx) = rest.find(|c: char| c == '_' || c == '.') {
        return rest[..idx].to_string();
    }

    // Split on camelCase boundary (lowercase → uppercase)
    let rchars: Vec<char> = rest.chars().collect();
    for i in 1..rchars.len() {
        if rchars[i].is_uppercase() && rchars[i - 1].is_lowercase() {
            return rest[..i].to_string();
        }
    }

    rest.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ResolvedSymbol;

    fn make_symbols() -> Vec<ResolvedSymbol> {
        vec![
            ResolvedSymbol {
                name: "func_a".into(),
                size: 100,
                source_path: Some("src/app/main.c".into()),
            },
            ResolvedSymbol {
                name: "func_b".into(),
                size: 200,
                source_path: Some("src/app/main.c".into()),
            },
            ResolvedSymbol {
                name: "func_c".into(),
                size: 50,
                source_path: Some("src/lib/util.c".into()),
            },
            ResolvedSymbol {
                name: "unknown_sym".into(),
                size: 30,
                source_path: None,
            },
        ]
    }

    #[test]
    fn test_root_size_is_total() {
        let tree = build_tree(&make_symbols());
        assert_eq!(tree.size, 380);
    }

    #[test]
    fn test_directory_hierarchy() {
        let tree = build_tree(&make_symbols());
        let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"src"), "should have src dir: {names:?}");
        assert!(
            names.iter().any(|n| n.starts_with("<unknown>")),
            "should have <unknown>: {names:?}"
        );
    }

    #[test]
    fn test_file_contains_symbols() {
        let tree = build_tree(&make_symbols());
        let src = tree.children.iter().find(|c| c.name == "src").unwrap();
        // app/main.c collapsed (app had single child main.c)
        let main_c = src
            .children
            .iter()
            .find(|c| c.name == "app/main.c")
            .unwrap();
        assert_eq!(main_c.size, 300);
        assert_eq!(main_c.children.len(), 2);
        let sym_names: Vec<&str> = main_c.children.iter().map(|c| c.name.as_str()).collect();
        assert!(sym_names.contains(&"func_a"));
        assert!(sym_names.contains(&"func_b"));
    }

    #[test]
    fn test_unknown_bucket() {
        let tree = build_tree(&make_symbols());
        // Single unknown sym is a singleton → lands in <other>, collapsed to <unknown>/<other>
        let unknown = tree.children.iter().find(|c| c.name.starts_with("<unknown>")).unwrap();
        assert_eq!(unknown.size, 30);
    }

    #[test]
    fn test_children_sorted_by_size_desc() {
        let tree = build_tree(&make_symbols());
        assert_eq!(tree.children[0].name, "src");
        assert!(tree.children[1].name.starts_with("<unknown>"));
    }

    #[test]
    fn test_empty_input() {
        let tree = build_tree(&[]);
        assert_eq!(tree.size, 0);
        assert!(tree.children.is_empty());
    }

    #[test]
    fn test_collapse_single_child_chain() {
        // a/b/c/file.c where a→b→c are single-child dirs
        let syms = vec![ResolvedSymbol {
            name: "func".into(),
            size: 42,
            source_path: Some("a/b/c/file.c".into()),
        }];
        let tree = build_tree(&syms);
        // Root should collapse a/b/c into one node
        assert_eq!(tree.children.len(), 1);
        let collapsed = &tree.children[0];
        assert_eq!(collapsed.name, "a/b/c/file.c");
        assert_eq!(collapsed.children.len(), 1);
        assert_eq!(collapsed.children[0].name, "func");
    }

    #[test]
    fn test_no_collapse_when_multiple_children() {
        // Two files in same dir → should NOT collapse
        let syms = vec![
            ResolvedSymbol {
                name: "f1".into(),
                size: 10,
                source_path: Some("dir/a.c".into()),
            },
            ResolvedSymbol {
                name: "f2".into(),
                size: 20,
                source_path: Some("dir/b.c".into()),
            },
        ];
        let tree = build_tree(&syms);
        let dir = &tree.children[0];
        assert_eq!(dir.name, "dir");
        assert_eq!(dir.children.len(), 2);
    }

    #[test]
    fn test_cluster_unknown_hungarian_merges() {
        let syms = vec![
            ResolvedSymbol { name: "Vitals_Init".into(), size: 100, source_path: None },
            ResolvedSymbol { name: "g_vitals_keys".into(), size: 50, source_path: None },
            ResolvedSymbol { name: "gp_vitals_ptr".into(), size: 30, source_path: None },
        ];
        let tree = build_tree(&syms);
        let unknown = tree.children.iter().find(|c| c.name == "<unknown>").unwrap();

        // "vitals" cluster: g_vitals_keys + gp_vitals_ptr (2 syms)
        // "Vitals" singleton → <other>
        let names: Vec<&str> = unknown.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"vitals"), "expected vitals cluster: {names:?}");
        let vitals = unknown.children.iter().find(|c| c.name == "vitals").unwrap();
        assert_eq!(vitals.children.len(), 2);
        assert_eq!(vitals.size, 80);
    }

    #[test]
    fn test_cluster_unknown_singletons_go_to_other() {
        let syms = vec![
            ResolvedSymbol { name: "mgfx_font_a".into(), size: 100, source_path: None },
            ResolvedSymbol { name: "mgfx_font_b".into(), size: 200, source_path: None },
            ResolvedSymbol { name: "strcmp".into(), size: 50, source_path: None },
            ResolvedSymbol { name: "sin".into(), size: 30, source_path: None },
        ];
        let tree = build_tree(&syms);
        let unknown = tree.children.iter().find(|c| c.name == "<unknown>").unwrap();

        // Should have "mgfx" cluster + "<other>" (strcmp and sin are singletons)
        let names: Vec<&str> = unknown.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"mgfx"), "expected mgfx: {names:?}");
        assert!(names.contains(&"<other>"), "expected <other>: {names:?}");
        assert_eq!(unknown.children.len(), 2);

        let other = unknown.children.iter().find(|c| c.name == "<other>").unwrap();
        assert_eq!(other.children.len(), 2);
        assert_eq!(other.size, 80);
    }

    #[test]
    fn test_cluster_unknown_groups_by_prefix() {
        let syms = vec![
            ResolvedSymbol { name: "mgfx_font_a".into(), size: 100, source_path: None },
            ResolvedSymbol { name: "mgfx_font_b".into(), size: 200, source_path: None },
            ResolvedSymbol { name: "__aeabi_dmul".into(), size: 50, source_path: None },
            ResolvedSymbol { name: "__aeabi_ddiv".into(), size: 60, source_path: None },
        ];
        let tree = build_tree(&syms);
        let unknown = tree.children.iter().find(|c| c.name == "<unknown>").unwrap();

        // Should have 2 cluster nodes: "mgfx" and "__"
        let names: Vec<&str> = unknown.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(unknown.children.len(), 2);
        assert!(names.contains(&"mgfx"), "expected mgfx cluster: {names:?}");
        assert!(names.contains(&"__"), "expected __ cluster: {names:?}");

        // mgfx cluster should contain the 2 mgfx symbols
        let mgfx = unknown.children.iter().find(|c| c.name == "mgfx").unwrap();
        assert_eq!(mgfx.size, 300);
        assert_eq!(mgfx.children.len(), 2);

        // __ cluster should contain the 2 aeabi symbols
        let dunder = unknown.children.iter().find(|c| c.name == "__").unwrap();
        assert_eq!(dunder.size, 110);
        assert_eq!(dunder.children.len(), 2);
    }

    #[test]
    fn test_extract_prefix_double_underscore() {
        assert_eq!(extract_prefix("__aeabi_dmul"), "__");
        assert_eq!(extract_prefix("__ieee754_powf"), "__");
        assert_eq!(extract_prefix("__kernel_cos"), "__");
    }

    #[test]
    fn test_extract_prefix_single_underscore() {
        assert_eq!(extract_prefix("_vfprintf_r"), "_");
        assert_eq!(extract_prefix("_malloc_r"), "_");
        assert_eq!(extract_prefix("_strtod_l"), "_");
    }

    #[test]
    fn test_extract_prefix_hungarian_underscore() {
        // g_vitals_keys → strip g_ → "vitals"
        assert_eq!(extract_prefix("g_vitals_keys"), "vitals");
        // s_buffer_ptr → strip s_ → "buffer"
        assert_eq!(extract_prefix("s_buffer_ptr"), "buffer");
    }

    #[test]
    fn test_extract_prefix_hungarian_camel() {
        // aGpioConfigList → strip a → "Gpio" (camelCase split on GpioConfigList)
        assert_eq!(extract_prefix("aGpioConfigList"), "Gpio");
    }

    #[test]
    fn test_extract_prefix_gp_hungarian() {
        // gp_bond_sync → strip gp_ → "bond"
        assert_eq!(extract_prefix("gp_bond_sync"), "bond");
    }

    #[test]
    fn test_extract_prefix_underscore_split() {
        assert_eq!(extract_prefix("mgfx_carousel_update"), "mgfx");
        assert_eq!(extract_prefix("bond_action_str"), "bond");
        assert_eq!(extract_prefix("CSWTCH.2"), "CSWTCH");
        assert_eq!(extract_prefix("IS31FL3763_LED_ADDRESS_H"), "IS31FL3763");
    }

    #[test]
    fn test_extract_prefix_camel_case_split() {
        // No underscore or dot — split on camelCase boundary
        assert_eq!(extract_prefix("localtime"), "localtime");
        assert_eq!(extract_prefix("brainpoolP256r1"), "brainpool");
    }

    #[test]
    fn test_extract_prefix_no_split() {
        // All uppercase or no boundary found — whole name
        assert_eq!(extract_prefix("strcmp"), "strcmp");
        assert_eq!(extract_prefix("sin"), "sin");
        assert_eq!(extract_prefix("K"), "K");
    }

    #[test]
    fn test_flatten_paths() {
        let tree = build_tree(&make_symbols());
        let paths = super::flatten_paths(&tree);
        // func_a (100) and func_b (200) are leaves under src/app/main.c
        assert_eq!(paths.get("src/app/main.c/func_b"), Some(&200));
        assert_eq!(paths.get("src/app/main.c/func_a"), Some(&100));
        assert_eq!(paths.get("src/lib/util.c/func_c"), Some(&50));
        // Non-leaf nodes should NOT be in the map
        assert!(!paths.contains_key("src"));
        assert!(!paths.contains_key("src/app/main.c"));
    }
}
