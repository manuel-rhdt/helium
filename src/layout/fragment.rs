#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Margin {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fragment {
    pub margin: Margin,
    pub class: SpecificFragment,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpecificFragment {
    Generic,
    Text(String),
}

impl Default for SpecificFragment {
    fn default() -> SpecificFragment {
        SpecificFragment::Generic
    }
}
