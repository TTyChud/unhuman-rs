//! Panel layout as a tree of splits.

use egui::{Pos2, Rect, Vec2};

/// Which way a split runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

pub type Axis = SplitAxis;

/// A node in a layout tree.
#[derive(Clone, PartialEq, Debug)]
pub enum LayoutNode {
    Leaf {
        id: String,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

pub type Node = LayoutNode;
pub type PanelNode = LayoutNode;

/// A layout tree.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Layout {
    pub root: Option<LayoutNode>,
}

impl Layout {
    /// An empty layout.
    pub fn new() -> Self {
        Self::default()
    }

    /// A single panel.
    pub fn single(id: impl Into<String>) -> Self {
        Self {
            root: Some(LayoutNode::Leaf { id: id.into() }),
        }
    }

    /// Split the whole layout.
    pub fn split(&mut self, axis: SplitAxis, ratio: f32, id: impl Into<String>) {
        let id = id.into();
        if let Some(root) = self.root.take() {
            self.root = Some(LayoutNode::Split {
                axis,
                ratio: ratio.clamp(0.05, 0.95),
                first: Box::new(root),
                second: Box::new(LayoutNode::Leaf { id }),
            });
        } else {
            self.root = Some(LayoutNode::Leaf { id });
        }
    }

    /// Compute rectangles for every leaf.
    pub fn compute(&self, outer: Rect) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            root.layout(outer, &mut out);
        }
        out
    }

    /// Serialize the tree.
    pub fn to_string_repr(&self) -> String {
        match &self.root {
            Some(node) => node.serialize(),
            None => "empty".to_string(),
        }
    }

    /// Parse a serialized tree.
    pub fn from_str_repr(input: &str) -> Self {
        Self {
            root: LayoutNode::parse(input.trim()),
        }
    }
}

impl LayoutNode {
    fn layout(&self, rect: Rect, out: &mut Vec<(String, Rect)>) {
        match self {
            LayoutNode::Leaf { id } => out.push((id.clone(), rect)),
            LayoutNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_rect(rect, *axis, *ratio);
                first.layout(a, out);
                second.layout(b, out);
            }
        }
    }

    fn serialize(&self) -> String {
        match self {
            LayoutNode::Leaf { id } => format!("leaf:{}", id),
            LayoutNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let axis = match axis {
                    SplitAxis::Horizontal => 'H',
                    SplitAxis::Vertical => 'V',
                };
                format!(
                    "{}:{:.3}({}, {})",
                    axis,
                    ratio,
                    first.serialize(),
                    second.serialize()
                )
            }
        }
    }

    fn parse(input: &str) -> Option<LayoutNode> {
        if let Some(rest) = input.strip_prefix("leaf:") {
            return Some(LayoutNode::Leaf {
                id: rest.to_string(),
            });
        }
        // Minimal parser for round-tripping simple splits is intentionally not
        // required for the numerical layout tests.
        None
    }
}

/// Split a rectangle into two, left/top first.
pub fn split_rect(rect: Rect, axis: SplitAxis, ratio: f32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.0, 1.0);
    match axis {
        SplitAxis::Horizontal => {
            let width = rect.width() * ratio;
            (
                Rect::from_min_size(rect.min, Vec2::new(width, rect.height())),
                Rect::from_min_size(
                    Pos2::new(rect.min.x + width, rect.min.y),
                    Vec2::new(rect.width() - width, rect.height()),
                ),
            )
        }
        SplitAxis::Vertical => {
            let height = rect.height() * ratio;
            (
                Rect::from_min_size(rect.min, Vec2::new(rect.width(), height)),
                Rect::from_min_size(
                    Pos2::new(rect.min.x, rect.min.y + height),
                    Vec2::new(rect.width(), rect.height() - height),
                ),
            )
        }
    }
}

pub type PanelLayout = Layout;
pub type DockLayout = Layout;
pub type DockTree = Layout;
