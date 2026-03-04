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
    root
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
        let app = src.children.iter().find(|c| c.name == "app").unwrap();
        let main_c = app.children.iter().find(|c| c.name == "main.c").unwrap();
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
}
