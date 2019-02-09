use super::flow::FlowBase;
use super::fragment::Fragment;

pub struct InlineFlow {
    base: FlowBase,
    fragments: Vec<Fragment>,
    lines: Vec<()>,
}
