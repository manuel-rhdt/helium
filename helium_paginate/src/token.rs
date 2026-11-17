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

/// A cut of a container: where the next fragment of it continues, in the flow of the content of
/// the container. It is the place inside the content where the fragment starts: in front of an
/// item (`cut` is `None`) or inside a paragraph item, in front of the part of it that starts at
/// the cut. What the items are is up to the layout service; the kernel only stores the cut and
/// compares the cuts of one container. They are in the order of the content.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerCut {
    pub item: u32,
    pub cut: Option<Cut>,
    /// The item of the first float in the content that is called for in front of the cut and
    /// waits for a column to be placed in, if there is one. The floats that wait are the floats of
    /// the content from this item to the cut: floats keep their order, so the ones that were
    /// placed are the first ones. It makes a fragment that ends with floats that wait another
    /// place than one that ends where none does.
    pub waiting: Option<u32>,
}

impl ContainerCut {
    /// In front of an item of the content.
    pub fn at_item(item: usize) -> Self {
        ContainerCut { item: item as u32, cut: None, waiting: None }
    }

    /// The same place, with the floats that wait from the item `waiting` on.
    pub fn with_waiting(self, waiting: Option<usize>) -> Self {
        ContainerCut { waiting: waiting.map(|item| item as u32), ..self }
    }

    /// The position in the content that the cut is.
    pub fn position(self) -> Position {
        match self.cut {
            None => Position::Item(self.item as usize),
            Some(cut) => Position::InParagraph { item: self.item as usize, cut },
        }
    }

    /// The cut of a position of the content.
    pub fn of(position: Position) -> Self {
        match position {
            Position::Item(item) => ContainerCut::at_item(item),
            Position::InParagraph { item, cut } => ContainerCut { item: item as u32, cut: Some(cut), waiting: None },
            Position::InContainer { .. } => panic!("a container is not inside a container"),
        }
    }

    fn order(self) -> u64 {
        (u64::from(self.item) << 32) | self.cut.map_or(0, |cut| u64::from(cut.0) + 1)
    }
}

/// A position in the flow, which a [`BreakToken`] stands for. Equal positions have equal tokens.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Position {
    /// In front of an item of the flow. The position behind the last item is the end of the
    /// document.
    Item(usize),
    /// Inside a paragraph of the flow, in front of the part of it that starts at the cut.
    InParagraph { item: usize, cut: Cut },
    /// Inside a container that is split, in front of the fragment of it that starts at the cut.
    InContainer { item: usize, cut: ContainerCut },
}

impl Position {
    /// The place of the position in the flow: positions that come first in the document compare
    /// less. The start of a paragraph or of a container item is in front of the cuts inside it.
    pub fn order(self) -> (usize, u64) {
        match self {
            Position::Item(item) => (item, 0),
            Position::InParagraph { item, cut } => (item, u64::from(cut.0) + 1),
            Position::InContainer { item, cut } => (item, cut.order() + 1),
        }
    }

    /// The item the position is in front of or inside of.
    pub fn item(self) -> usize {
        match self {
            Position::Item(item) | Position::InParagraph { item, .. } | Position::InContainer { item, .. } => item,
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
