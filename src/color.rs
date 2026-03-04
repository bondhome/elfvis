/// An RGB color.
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn to_css(&self) -> String {
        format!("rgb({},{},{})", self.r, self.g, self.b)
    }
}

/// Generate a pastel color for a given color group and depth.
/// - `group`: top-level ancestor index (determines hue)
/// - `num_groups`: total number of top-level groups (for hue spacing)
/// - `depth`: depth in the tree (deeper = slightly darker)
pub fn pastel_color(group: usize, num_groups: usize, depth: usize) -> Color {
    let num_groups = num_groups.max(1);
    let hue = (group as f64 / num_groups as f64) * 360.0;
    let saturation = 0.40;
    let lightness = (0.85 - depth as f64 * 0.03).max(0.65);

    hsl_to_rgb(hue, saturation, lightness)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Color {
        r: ((r1 + m) * 255.0).round() as u8,
        g: ((g1 + m) * 255.0).round() as u8,
        b: ((b1 + m) * 255.0).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_produces_pastel_range() {
        let c = pastel_color(0, 5, 1);
        assert!(
            c.r > 150 && c.g > 150 && c.b > 150,
            "should be pastel, got ({},{},{})",
            c.r,
            c.g,
            c.b
        );
    }

    #[test]
    fn test_different_groups_different_hues() {
        let c0 = pastel_color(0, 4, 1);
        let c1 = pastel_color(1, 4, 1);
        let diff = (c0.r as i32 - c1.r as i32).abs()
            + (c0.g as i32 - c1.g as i32).abs()
            + (c0.b as i32 - c1.b as i32).abs();
        assert!(
            diff > 30,
            "different groups should have visually distinct colors, diff={diff}"
        );
    }

    #[test]
    fn test_deeper_is_slightly_darker() {
        let shallow = pastel_color(0, 4, 1);
        let deep = pastel_color(0, 4, 4);
        let lum_shallow = shallow.r as u32 + shallow.g as u32 + shallow.b as u32;
        let lum_deep = deep.r as u32 + deep.g as u32 + deep.b as u32;
        assert!(lum_shallow > lum_deep, "deeper nodes should be slightly darker");
    }

    #[test]
    fn test_single_group() {
        let c = pastel_color(0, 1, 0);
        assert!(c.r > 100);
    }
}
