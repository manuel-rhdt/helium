use std::str::FromStr;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline
}

impl FromStr for Display {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Display::None),
            "block" => Ok(Display::Block),
            "inline" => Ok(Display::Inline),
            _ => Err(())
        }
    }
}