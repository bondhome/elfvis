use elfvis::parse::parse_elf;
use elfvis::tree::build_tree;
use elfvis::layout::layout;

static ARM_ELF: &[u8] = include_bytes!("fixtures/arm.elf");

#[test]
fn test_full_pipeline_arm() {
    let symbols = parse_elf(ARM_ELF).unwrap();
    assert!(!symbols.is_empty(), "should parse symbols from ARM ELF");

    let tree = build_tree(&symbols);
    assert!(tree.size > 0, "tree should have nonzero total size");
    assert!(!tree.children.is_empty(), "tree should have children");

    let root = layout(&tree, 1024.0, 768.0);
    assert_eq!(root.rect.w, 1024.0);
    assert_eq!(root.rect.h, 768.0);
    assert!(!root.children.is_empty(), "layout should have children");

    // Every leaf should have a valid rect
    fn check_leaves(node: &elfvis::layout::LayoutNode) {
        if node.is_leaf {
            assert!(node.rect.w >= 0.0, "leaf width should be >= 0");
            assert!(node.rect.h >= 0.0, "leaf height should be >= 0");
        }
        for child in &node.children {
            check_leaves(child);
        }
    }
    check_leaves(&root);
}
