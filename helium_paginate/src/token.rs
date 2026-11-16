use std::collections::HashMap;

/// Where a page starts, as a handle that is cheap to copy, compare and store.
///
/// A token is opaque. Only the kernel creates tokens: it numbers the places where a page can
/// start in the order of the flow, so a token compares less than every token that can follow it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BreakToken(pub(crate) u32);

/// A position in the flow, which a [`BreakToken`] stands for. Equal positions have equal tokens.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Position {
    /// In front of an item of the flow. The position behind the last item is the end of the
    /// document.
    Item(usize),
    /// Inside a paragraph of the flow, in front of a line of it (not the first).
    InParagraph { item: usize, line: u32 },
    // `InContainer`, a position inside a container that is split, comes with the containers.
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
}
