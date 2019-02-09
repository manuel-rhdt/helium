use crate::parser::ParsedData;
use crate::tree::{NodeData, NodeRef};

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Length {
    Px(f32),
}

impl Length {
    pub fn to_px(self) -> f32 {
        match self {
            Length::Px(px) => px,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LengthSpec {
    Auto,
    Length(Length),
}

impl Default for LengthSpec {
    fn default() -> LengthSpec {
        LengthSpec::Auto
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PageSize {
    Auto,
    Size { width: Length, height: Length },
}

impl Default for PageSize {
    fn default() -> PageSize {
        PageSize::Auto
    }
}

/// Creates a builder for a struct automatically. Requires the struct to implement `Default`.
macro_rules! create_builder {
    (
        $builder_name:ident,
        $(#[$attrs:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$fattrs:meta])*
                $fvis:vis $fname:ident: $ftype:ty
            ),*
            $(,)*
        }
    ) => {
        $(#[$attrs])*
        $vis struct $name {
            $(
                $(#[$fattrs])*
                $fvis $fname: $ftype
            ),*
        }

        #[derive(Debug)]
        $vis struct $builder_name {
            $($fvis $fname: Option<$ftype>),*
        }

        impl $builder_name {
            $vis fn new() -> Self {
                Self {
                    $(
                        $fname: None,
                    )*
                }
            }

            $(
                $fvis fn $fname(mut self, val: $ftype) -> Self {
                    self.$fname = Some(val);
                    self
                }
            )*
            fn __build(self) -> $name {
                let mut product: $name = Default::default();
                $(
                   if let Some(val) = self.$fname {
                       product.$fname = val;
                   }
                )*
                product
            }
        }
    };
}

create_builder!(
    StyleBuilder,
    #[derive(Debug, Copy, Clone, Default)]
    pub struct Style {
        pub display: Display,
        /// Describes the page size
        pub size: PageSize,

        pub width: LengthSpec,
        pub height: LengthSpec,
    }
);

impl StyleBuilder {
    /// Creates the Style and computes computed properties
    pub fn build(mut self) -> Style {
        if let (None, Some(PageSize::Size { width, .. })) = (self.width, self.size) {
            self.width = Some(LengthSpec::Length(width));
        }
        if let (None, Some(PageSize::Size { height, .. })) = (self.height, self.size) {
            self.height = Some(LengthSpec::Length(height));
        }
        self.__build()
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Display {
    Block,
    Inline,
}

impl Default for Display {
    fn default() -> Display {
        Display::Block
    }
}

fn display_default(element_name: &str) -> Display {
    match element_name {
        _ => Display::Block,
    }
}

pub fn style_for_element(elem: NodeRef<NodeData>) -> Style {
    match &elem.html_data {
        ParsedData::Element(element_data) => Style {
            display: display_default(&element_data.name.local),
            ..Default::default()
        },
        ParsedData::Text(_) => Style {
            display: Display::Inline,
            ..Default::default()
        },
        _ => Style::default(),
    }
}
