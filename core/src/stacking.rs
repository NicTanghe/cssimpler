use crate::{MixBlendMode, RenderNode, ZIndex};

pub fn establishes_stacking_context(node: &RenderNode) -> bool {
    node.style.cache_layer
        || node.style.isolate_blending
        || node.style.mix_blend_mode != MixBlendMode::Normal
        || (node.style.positioned && matches!(node.style.z_index, ZIndex::Integer(_)))
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
