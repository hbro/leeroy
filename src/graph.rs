//! Box drawing for the run view, on a character grid. No IO.
//!
//! A run is a tree of builds (each triggered by one other). The boxes view
//! stacks them top to bottom in tree order, each indented one step under the
//! build that triggered it, with a line from that build's box into its own:
//!
//! ```text
//! ┌──────────┐
//! │ ● app #1 │
//! └┬─────────┘
//!  │  ┌─────────────┐
//!  ├─▶│ ● deploy #4 │
//!  │  └─────────────┘
//!  │  ┌──────────┐
//!  └─▶│ ● e2e #2 │
//!     └──────────┘
//! ```

/// Line directions of a cell, merged into one box-drawing character.
pub const UP: u8 = 1;
pub const DOWN: u8 = 2;
pub const LEFT: u8 = 4;
pub const RIGHT: u8 = 8;

/// One character cell of the drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cell {
    #[default]
    Empty,
    /// Part of node `node`'s box border.
    Border { node: usize, dirs: u8 },
    /// Part of a connecting line.
    Line(u8),
    /// Arrowhead, just left of the box it points at.
    Arrow,
}

/// The box-drawing character for a set of directions.
pub fn line_char(dirs: u8) -> char {
    match dirs {
        d if d == LEFT | RIGHT || d == LEFT || d == RIGHT => '─',
        d if d == UP | DOWN || d == UP || d == DOWN => '│',
        d if d == DOWN | RIGHT => '┌',
        d if d == DOWN | LEFT => '┐',
        d if d == UP | RIGHT => '└',
        d if d == UP | LEFT => '┘',
        d if d == UP | DOWN | RIGHT => '├',
        d if d == UP | DOWN | LEFT => '┤',
        d if d == LEFT | RIGHT | DOWN => '┬',
        d if d == LEFT | RIGHT | UP => '┴',
        d if d == UP | DOWN | LEFT | RIGHT => '┼',
        _ => ' ',
    }
}

/// Where a node's box ended up. `y` is the top border; the label goes on
/// row `y + 1`, starting at `x + 2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeBox {
    pub x: usize,
    pub y: usize,
    pub width: usize,
}

/// Box height: border, label, border.
pub const BOX_HEIGHT: usize = 3;

/// Columns a box is indented under the one that triggered it.
pub const INDENT: usize = 4;

/// Result of [`tree_layout`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    /// `height` rows of `width` cells.
    pub cells: Vec<Vec<Cell>>,
    /// Per input node, in input order.
    pub boxes: Vec<NodeBox>,
}

/// Stack the nodes of a tree as boxes: `widths` are label widths in
/// columns, `parents[i]` the node that node `i` hangs under. Nodes must be in
/// tree order (depth first), so every parent comes before its children.
pub fn tree_layout(widths: &[usize], parents: &[Option<usize>]) -> Layout {
    let n = widths.len();
    let parent = |i: usize| parents.get(i).copied().flatten().filter(|&p| p < i);
    let mut depth = vec![0usize; n];
    for i in 0..n {
        if let Some(p) = parent(i) {
            depth[i] = depth[p] + 1;
        }
    }
    let boxes: Vec<NodeBox> = (0..n)
        .map(|i| NodeBox {
            x: depth[i] * INDENT,
            y: i * BOX_HEIGHT,
            width: widths[i] + 4,
        })
        .collect();
    let width = boxes.iter().map(|b| b.x + b.width).max().unwrap_or(0);
    let height = n * BOX_HEIGHT;
    let mut cells = vec![vec![Cell::Empty; width]; height];

    for (node, b) in boxes.iter().enumerate() {
        let right = b.x + b.width - 1;
        // Top and bottom border: corners at the ends, lines in between.
        let edge = |x: usize, left_corner: u8, right_corner: u8| match x {
            x if x == b.x => left_corner,
            x if x == right => right_corner,
            _ => LEFT | RIGHT,
        };
        for (x, cell) in (b.x..).zip(&mut cells[b.y][b.x..=right]) {
            let dirs = edge(x, DOWN | RIGHT, DOWN | LEFT);
            *cell = Cell::Border { node, dirs };
        }
        for (x, cell) in (b.x..).zip(&mut cells[b.y + 2][b.x..=right]) {
            let dirs = edge(x, UP | RIGHT, UP | LEFT);
            *cell = Cell::Border { node, dirs };
        }
        for x in [b.x, right] {
            cells[b.y + 1][x] = Cell::Border {
                node,
                dirs: UP | DOWN,
            };
        }
    }

    let mut add = |x: usize, y: usize, dirs: u8| {
        let cell = &mut cells[y][x];
        *cell = match *cell {
            Cell::Border { node, dirs: d } => Cell::Border {
                node,
                dirs: d | dirs,
            },
            Cell::Line(d) => Cell::Line(d | dirs),
            Cell::Empty => Cell::Line(dirs),
            Cell::Arrow => Cell::Arrow,
        };
    };
    let mut arrows = Vec::new();
    for (child, b) in boxes.iter().enumerate() {
        let Some(p) = parent(child).map(|p| boxes[p]) else {
            continue;
        };
        let (x, port) = (p.x + 1, b.y + 1);
        add(x, p.y + 2, DOWN); // out of the parent's bottom border
        for y in p.y + 3..port {
            add(x, y, UP | DOWN);
        }
        add(x, port, UP | RIGHT);
        add(x + 1, port, LEFT | RIGHT);
        arrows.push((x + 2, port));
    }
    for (x, y) in arrows {
        cells[y][x] = Cell::Arrow;
    }

    Layout {
        width,
        height,
        cells,
        boxes,
    }
}

/// The drawing as text (label areas blank), for tests and debugging.
pub fn to_text(layout: &Layout) -> Vec<String> {
    layout
        .cells
        .iter()
        .map(|row| {
            let line: String = row
                .iter()
                .map(|cell| match cell {
                    Cell::Empty => ' ',
                    Cell::Border { dirs, .. } | Cell::Line(dirs) => line_char(*dirs),
                    Cell::Arrow => '▶',
                })
                .collect();
            line.trim_end().to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text with labels (node index as a letter) filled in.
    fn picture(widths: &[usize], parents: &[Option<usize>]) -> String {
        let layout = tree_layout(widths, parents);
        let mut rows: Vec<Vec<char>> = to_text(&layout)
            .iter()
            .map(|r| {
                let mut chars: Vec<char> = r.chars().collect();
                chars.resize(layout.width, ' ');
                chars
            })
            .collect();
        for (i, b) in layout.boxes.iter().enumerate() {
            for dx in 0..widths[i] {
                rows[b.y + 1][b.x + 2 + dx] = (b'a' + i as u8) as char;
            }
        }
        rows.iter()
            .map(|r| r.iter().collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_tree_of_boxes() {
        // a triggered b and d; b triggered c.
        let text = picture(&[1, 2, 1, 1], &[None, Some(0), Some(1), Some(0)]);
        assert_eq!(
            text,
            "\
┌───┐
│ a │
└┬──┘
 │  ┌────┐
 ├─▶│ bb │
 │  └┬───┘
 │   │  ┌───┐
 │   └─▶│ c │
 │      └───┘
 │  ┌───┐
 └─▶│ d │
    └───┘"
        );
    }

    #[test]
    fn a_single_build() {
        assert_eq!(picture(&[1], &[None]), "┌───┐\n│ a │\n└───┘");
    }

    #[test]
    fn junction_characters() {
        assert_eq!(line_char(UP | DOWN | RIGHT), '├');
        assert_eq!(line_char(LEFT | RIGHT | DOWN), '┬');
        assert_eq!(line_char(UP | DOWN | LEFT | RIGHT), '┼');
        assert_eq!(line_char(RIGHT), '─');
    }
}
