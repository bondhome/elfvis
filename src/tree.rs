/// A node in the size tree.
#[derive(Debug, Clone)]
pub struct SizeNode {
    /// Name of this node (directory name, filename, or symbol name).
    pub name: String,
    /// Total size of this node and all descendants (bytes).
    pub size: u64,
    /// Child nodes. Empty for leaf (symbol) nodes.
    pub children: Vec<SizeNode>,
}

/// Build a size tree from resolved symbols.
/// Paths are split on '/' to create the directory hierarchy.
/// Symbols without a source path go under "<unknown>".
pub fn build_tree(symbols: &[crate::parse::ResolvedSymbol]) -> SizeNode {
    let mut root = SizeNode {
        name: String::new(),
        size: 0,
        children: Vec::new(),
    };

    for sym in symbols {
        let path = match &sym.source_path {
            Some(p) => p.as_str(),
            None => "<unknown>",
        };

        // Split path into components, append symbol name as leaf
        let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        parts.push(&sym.name);

        // Walk/create the tree
        let mut node = &mut root;
        for part in &parts[..parts.len() - 1] {
            let idx = node.children.iter().position(|c| c.name == *part);
            let idx = match idx {
                Some(i) => i,
                None => {
                    node.children.push(SizeNode {
                        name: part.to_string(),
                        size: 0,
                        children: Vec::new(),
                    });
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
        }

        // Add leaf symbol
        node.children.push(SizeNode {
            name: parts.last().unwrap().to_string(),
            size: sym.size,
            children: Vec::new(),
        });
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
    node.children.sort_by(|a, b| b.size.cmp(&a.size));
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
fn extract_prefix(name: &str) -> String {
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
            names.contains(&"<unknown>"),
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
        let unknown = tree.children.iter().find(|c| c.name == "<unknown>").unwrap();
        assert_eq!(unknown.size, 30);
    }

    #[test]
    fn test_children_sorted_by_size_desc() {
        let tree = build_tree(&make_symbols());
        assert_eq!(tree.children[0].name, "src");
        assert_eq!(tree.children[1].name, "<unknown>");
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
}
