use crate::{RenderNode, ZIndex};

pub fn establishes_stacking_context(node: &RenderNode) -> bool {
    node.style.positioned && matches!(node.style.z_index, ZIndex::Integer(_))
}

pub fn stacking_context_level(node: &RenderNode) -> i32 {
    if !establishes_stacking_context(node) {
        return 0;
    }

    match node.style.z_index {
        ZIndex::Auto => 0,
        ZIndex::Integer(value) => value,
    }
}
