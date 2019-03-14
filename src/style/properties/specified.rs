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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Unit {
    Px,
    Cm
}

impl FromStr for Unit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "px" => Ok(Unit::Px),
            "cm" => Ok(Unit::Cm),
            _ => Err(())
        }
    }
}