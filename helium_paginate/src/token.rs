use std::collections::HashMap;

/// Where a page starts, as a handle that is cheap to copy, compare and store.
///
/// A token is opaque. Only the kernel creates tokens. Two tokens are equal if and only if they
/// stand for the same [`Position`]. Tokens have no order; the order of the places in the flow is
/// the order of their positions ([`Position::order`]).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BreakToken(pub(crate) u32);

/// A cut of a paragraph: where the next page continues it, as the index of a place where the
/// paragraph can be broken. What a cut means is up to the layout service; the kernel only
/// stores it and compares cuts of one paragraph. The cuts of a paragraph are in the order of the
/// paragraph.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cut(pub u32);

/// A position in the flow, which a [`BreakToken`] stands for. Equal positions have equal tokens.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Position {
    /// In front of an item of the flow. The position behind the last item is the end of the
    /// document.
    Item(usize),
    /// Inside a paragraph of the flow, in front of the part of it that starts at the cut.
    InParagraph { item: usize, cut: Cut },
    // `InContainer`, a position inside a container that is split, comes with the containers.
}

impl Position {
    /// The place of the position in the flow: positions that come first in the document compare
    /// less. The start of a paragraph item is in front of the cuts inside it.
    pub fn order(self) -> (usize, u32) {
        match self {
            Position::Item(item) => (item, 0),
            Position::InParagraph { item, cut } => (item, cut.0 + 1),
        }
    }

    /// The item the position is in front of or inside of.
    pub fn item(self) -> usize {
        match self {
            Position::Item(item) | Position::InParagraph { item, .. } => item,
        }
    }
}

/// Hash-consing of positions.
#[derive(Debug, Default)]
pub(crate) struct Interner {
    positions: Vec<Position>,
    tokens: HashMap<Position, BreakToken>,
}

impl Interner {
    pub(crate) fn intern(&mut self, position: Position) -> BreakToken {
        if let Some(&token) = self.tokens.get(&position) {
            return token;
        }
        let token = BreakToken(self.positions.len() as u32);
        self.positions.push(position);
        self.tokens.insert(position, token);
        token
    }

    pub(crate) fn position(&self, token: BreakToken) -> Position {
        self.positions[token.0 as usize]
    }

    pub(crate) fn token(&self, position: Position) -> Option<BreakToken> {
        self.tokens.get(&position).copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.positions.len()
    }
}
