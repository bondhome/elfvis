use crate::tree::SizeNode;

/// A positioned rectangle in the treemap.
#[derive(Debug, Clone)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A laid-out node ready for rendering.
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub rect: Rect,
    pub name: String,
    pub size: u64,
    pub depth: usize,
    pub is_leaf: bool,
    pub hue: f64,
    pub children: Vec<LayoutNode>,
}

pub const HEADER_HEIGHT: f64 = 18.0;
pub const MIN_HEADER_HEIGHT: f64 = 24.0;
pub const PADDING: f64 = 1.0;

/// Lay out a `SizeNode` tree as a squarified treemap within the given canvas dimensions.
pub fn layout(tree: &SizeNode, width: f64, height: f64) -> LayoutNode {
    let rect = Rect { x: 0.0, y: 0.0, w: width, h: height };
    layout_node(tree, &rect, 0, 0.0, 360.0)
}

fn layout_node(node: &SizeNode, rect: &Rect, depth: usize, hue_start: f64, hue_end: f64) -> LayoutNode {
    let is_leaf = node.children.is_empty();
    let hue = (hue_start + hue_end) / 2.0;

    let children = if is_leaf || node.size == 0 {
        Vec::new()
    } else {
        // Reserve header space for non-root directory nodes when tall enough.
        let show_header = rect.h >= MIN_HEADER_HEIGHT && depth > 0;
        let header_offset = if show_header { HEADER_HEIGHT } else { 0.0 };
        let inner = Rect {
            x: rect.x + PADDING,
            y: rect.y + header_offset + PADDING,
            w: (rect.w - 2.0 * PADDING).max(0.0),
            h: (rect.h - header_offset - 2.0 * PADDING).max(0.0),
        };

        let sizes: Vec<f64> = node.children.iter().map(|c| c.size as f64).collect();
        let rects = squarify(&sizes, &inner);

        // Subdivide this node's hue interval among children proportional to size.
        let total: f64 = sizes.iter().sum();
        let mut cursor = hue_start;

        node.children
            .iter()
            .zip(rects.iter())
            .zip(sizes.iter())
            .map(|((child, r), &child_size)| {
                let span = if total > 0.0 {
                    (child_size / total) * (hue_end - hue_start)
                } else {
                    0.0
                };
                let child_hue_start = cursor;
                cursor += span;
                layout_node(child, r, depth + 1, child_hue_start, cursor)
            })
            .collect()
    };

    LayoutNode {
        rect: rect.clone(),
        name: node.name.clone(),
        size: node.size,
        depth,
        is_leaf,
        hue,
        children,
    }
}

/// Partition `sizes` into positioned rectangles within `rect` using the squarified algorithm.
fn squarify(sizes: &[f64], rect: &Rect) -> Vec<Rect> {
    if sizes.is_empty() {
        return Vec::new();
    }
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 {
        return sizes.iter().map(|_| rect.clone()).collect();
    }

    let mut results = vec![Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }; sizes.len()];
    squarify_recursive(sizes, rect, total, &mut results, 0);
    results
}

fn squarify_recursive(
    sizes: &[f64],
    rect: &Rect,
    total: f64,
    results: &mut [Rect],
    offset: usize,
) {
    if sizes.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    if sizes.len() == 1 {
        results[offset] = rect.clone();
        return;
    }

    let short_side = rect.w.min(rect.h);
    let area = rect.w * rect.h;

    // Greedily build a row: keep adding items while aspect ratio improves.
    let mut row: Vec<usize> = vec![0];
    let mut row_total = sizes[0];

    for i in 1..sizes.len() {
        let current_vals: Vec<f64> = row.iter().map(|&j| sizes[j]).collect();
        let current_worst = worst_aspect(&current_vals, short_side, total, area);

        let mut candidate_vals = current_vals;
        candidate_vals.push(sizes[i]);
        let candidate_worst = worst_aspect(&candidate_vals, short_side, total, area);

        if current_worst >= candidate_worst {
            row.push(i);
            row_total += sizes[i];
        } else {
            break;
        }
    }

    // Lay out the row along one dimension.
    let row_fraction = row_total / total;
    let horizontal = rect.w >= rect.h;
    let (row_rect, remaining_rect) = if horizontal {
        let w = rect.w * row_fraction;
        (
            Rect { x: rect.x, y: rect.y, w, h: rect.h },
            Rect { x: rect.x + w, y: rect.y, w: rect.w - w, h: rect.h },
        )
    } else {
        let h = rect.h * row_fraction;
        (
            Rect { x: rect.x, y: rect.y, w: rect.w, h },
            Rect { x: rect.x, y: rect.y + h, w: rect.w, h: rect.h - h },
        )
    };

    // Position each item within the row strip.
    let mut pos = 0.0;
    for &idx in &row {
        let frac = sizes[idx] / row_total;
        if horizontal {
            let h = row_rect.h * frac;
            results[offset + idx] = Rect { x: row_rect.x, y: row_rect.y + pos, w: row_rect.w, h };
            pos += h;
        } else {
            let w = row_rect.w * frac;
            results[offset + idx] = Rect { x: row_rect.x + pos, y: row_rect.y, w, h: row_rect.h };
            pos += w;
        }
    }

    // Recurse on the remaining items.
    let next_start = row.len();
    if next_start < sizes.len() {
        squarify_recursive(
            &sizes[next_start..],
            &remaining_rect,
            total - row_total,
            results,
            offset + next_start,
        );
    }
}

/// Compute the worst (maximum) aspect ratio among items in a candidate row.
fn worst_aspect(row: &[f64], side: f64, total: f64, area: f64) -> f64 {
    let row_sum: f64 = row.iter().sum();
    if row_sum <= 0.0 || side <= 0.0 || total <= 0.0 {
        return f64::MAX;
    }
    let row_area = area * (row_sum / total);
    let row_side = row_area / side;

    row.iter()
        .map(|&s| {
            let item_side = if row_side > 0.0 {
                (s / row_sum) * side
            } else {
                0.0
            };
            if item_side > 0.0 && row_side > 0.0 {
                (row_side / item_side).max(item_side / row_side)
            } else {
                f64::MAX
            }
        })
        .fold(0.0_f64, f64::max)
}

/// Find the deepest (most specific) node at the given point.
pub fn hit_test(node: &LayoutNode, x: f64, y: f64) -> Option<Vec<String>> {
    if x < node.rect.x || x > node.rect.x + node.rect.w
        || y < node.rect.y || y > node.rect.y + node.rect.h
    {
        return None;
    }

    for child in &node.children {
        if let Some(path) = hit_test(child, x, y) {
            let mut result = vec![node.name.clone()];
            result.extend(path);
            return Some(result);
        }
    }

    Some(vec![node.name.clone()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::SizeNode;

    fn sample_tree() -> SizeNode {
        SizeNode {
            name: "root".into(),
            size: 1000,
            children: vec![
                SizeNode {
                    name: "big".into(),
                    size: 700,
                    children: vec![
                        SizeNode { name: "a.c".into(), size: 400, children: vec![] },
                        SizeNode { name: "b.c".into(), size: 300, children: vec![] },
                    ],
                },
                SizeNode {
                    name: "small".into(),
                    size: 300,
                    children: vec![
                        SizeNode { name: "c.c".into(), size: 300, children: vec![] },
                    ],
                },
            ],
        }
    }

    #[test]
    fn test_root_fills_canvas() {
        let layout = layout(&sample_tree(), 800.0, 600.0);
        assert_eq!(layout.rect.x, 0.0);
        assert_eq!(layout.rect.y, 0.0);
        assert_eq!(layout.rect.w, 800.0);
        assert_eq!(layout.rect.h, 600.0);
    }

    #[test]
    fn test_children_fit_within_parent() {
        let root = layout(&sample_tree(), 800.0, 600.0);
        for child in &root.children {
            assert!(child.rect.x >= root.rect.x, "child x out of bounds");
            assert!(child.rect.y >= root.rect.y, "child y out of bounds");
            assert!(
                child.rect.x + child.rect.w <= root.rect.x + root.rect.w + 0.01,
                "child right edge out of bounds"
            );
            assert!(
                child.rect.y + child.rect.h <= root.rect.y + root.rect.h + 0.01,
                "child bottom edge out of bounds"
            );
        }
    }

    #[test]
    fn test_leaf_area_proportional_to_size() {
        let root = layout(&sample_tree(), 800.0, 600.0);
        let big = &root.children[0];
        let small = &root.children[1];
        let big_area = big.rect.w * big.rect.h;
        let small_area = small.rect.w * small.rect.h;
        let ratio = big_area / (big_area + small_area);
        assert!((ratio - 0.7).abs() < 0.05, "area ratio should be ~0.7, got {ratio}");
    }

    #[test]
    fn test_hue_proportional_to_size() {
        let root = layout(&sample_tree(), 800.0, 600.0);
        // "big" is 700/1000 = 70% → interval 0..252, midpoint 126
        // "small" is 300/1000 = 30% → interval 252..360, midpoint 306
        let big_hue = root.children[0].hue;
        let small_hue = root.children[1].hue;
        assert!((big_hue - 126.0).abs() < 0.01, "big midpoint should be ~126, got {big_hue}");
        assert!((small_hue - 306.0).abs() < 0.01, "small midpoint should be ~306, got {small_hue}");
    }

    #[test]
    fn test_hue_subdivides_recursively() {
        let root = layout(&sample_tree(), 800.0, 600.0);
        // "big" interval is 0..252
        // Its children: a.c=400 (4/7 of 252=144 → 0..144, mid 72)
        //               b.c=300 (3/7 of 252=108 → 144..252, mid 198)
        let a_hue = root.children[0].children[0].hue;
        let b_hue = root.children[0].children[1].hue;
        assert!((a_hue - 72.0).abs() < 0.1, "a.c midpoint should be ~72, got {a_hue}");
        assert!((b_hue - 198.0).abs() < 0.1, "b.c midpoint should be ~198, got {b_hue}");
    }

    #[test]
    fn test_zero_size_handled() {
        let tree = SizeNode { name: "root".into(), size: 0, children: vec![] };
        let root = layout(&tree, 800.0, 600.0);
        assert_eq!(root.rect.w, 800.0);
        assert!(root.children.is_empty());
    }
}
