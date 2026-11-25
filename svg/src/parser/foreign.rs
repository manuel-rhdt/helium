// This file is part of the Umbruch fork of usvg.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `foreignObject`: a rectangle with content that is not SVG.
//!
//! The content is not read here. The tree only records the rectangle and the handle of the
//! element (see [`crate::xml::DocumentBuilder::element`]), so that whoever draws the tree can
//! draw the content in the coordinate system of the rectangle.

use svgtypes::Length;

use super::converter;
use super::svgtree::{AId, SvgNode};
use crate::{ForeignObject, Group, Node, NonZeroRect};

pub(crate) fn convert(
    node: SvgNode,
    state: &converter::State,
    parent: &mut Group,
) -> Option<()> {
    let Some(handle) = node.handle() else {
        log::warn!("A foreignObject needs a document that knows its content. Skipped.");
        return None;
    };
    let x = node.convert_user_length(AId::X, state, Length::zero());
    let y = node.convert_user_length(AId::Y, state, Length::zero());
    let width = node.convert_user_length(AId::Width, state, Length::zero());
    let height = node.convert_user_length(AId::Height, state, Length::zero());
    let rect = NonZeroRect::from_xywh(x, y, width, height);
    let Some(rect) = rect else {
        log::warn!("A foreignObject has an invalid size. Skipped.");
        return None;
    };
    let abs_bounding_box = rect.transform(parent.abs_transform)?;
    parent.children.push(Node::Foreign(Box::new(ForeignObject {
        id: node.element_id().to_string(),
        handle,
        rect,
        abs_transform: parent.abs_transform,
        abs_bounding_box,
    })));
    Some(())
}
