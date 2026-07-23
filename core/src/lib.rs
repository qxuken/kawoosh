use std::collections::BTreeMap;

use slotmap::SlotMap;

pub struct Core {
    buffers: SlotMap<BufferId, text_buffer::Buffer>,
}

slotmap::new_key_type! {
    pub struct BufferId;
    pub struct HighlightId;
}

type BufferOffset = usize;

pub struct Buffer {
    data: SlotMap<BufferId, text_buffer::Buffer>,
    metadata: BTreeMap<BufferOffset, BufferMetadata>,
}

pub struct BufferMetadata {
    editable: bool,
    style: Option<HighlightId>,
}
