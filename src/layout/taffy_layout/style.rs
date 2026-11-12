//! Conversion of helium's computed style to Taffy's [`taffy::Style`].

use crate::style::properties::computed::{
    BoxSizing, ComputedValues, ContentAlignment, FlexDirection, FlexWrap, GenericSize, GridAutoFlow,
    GridPlacement, ItemAlignment, LengthPercentage, SelfAlignment, TrackList, TrackSize,
};
use crate::style::properties::specified::{Display, PositionSpec};
use taffy::style_helpers::{
    TaffyAuto, TaffyMaxContent, TaffyMinContent, auto, fr, length, line, max_content, min_content,
    percent, span,
};

use taffy::geometry::{Rect, Size};
use taffy::style::{
    BoxSizing as TBoxSizing, Dimension, LengthPercentage as TLengthPercentage, LengthPercentageAuto, Position,
};

fn length_percentage(value: LengthPercentage) -> TLengthPercentage {
    match value {
        LengthPercentage::PixelLength(px) => TLengthPercentage::length(px.0),
        LengthPercentage::Percentage(p) => TLengthPercentage::percent(p.0),
        LengthPercentage::Both { length, percentage } => {
            // TODO: map to a Taffy calc() value
            log::warn!(
                "length + percentage ({:?} + {:?}) is not supported by the Taffy layout, using the length",
                length,
                percentage
            );
            TLengthPercentage::length(length.0)
        }
    }
}

fn length_percentage_auto(value: GenericSize) -> LengthPercentageAuto {
    match value {
        GenericSize::Auto => LengthPercentageAuto::auto(),
        GenericSize::LengthPercentage(lp) => match lp {
            LengthPercentage::PixelLength(px) => LengthPercentageAuto::length(px.0),
            LengthPercentage::Percentage(p) => LengthPercentageAuto::percent(p.0),
            LengthPercentage::Both { .. } => LengthPercentageAuto::from(length_percentage(lp)),
        },
    }
}

fn dimension(value: GenericSize) -> Dimension {
    match value {
        GenericSize::Auto => Dimension::auto(),
        GenericSize::LengthPercentage(lp) => match lp {
            LengthPercentage::PixelLength(px) => Dimension::length(px.0),
            LengthPercentage::Percentage(p) => Dimension::percent(p.0),
            LengthPercentage::Both { .. } => Dimension::from(length_percentage(lp)),
        },
    }
}

fn content_alignment(value: ContentAlignment) -> taffy::AlignContent {
    match value {
        ContentAlignment::Normal => taffy::AlignContent::NORMAL,
        ContentAlignment::Start => taffy::AlignContent::START,
        ContentAlignment::End => taffy::AlignContent::END,
        ContentAlignment::FlexStart => taffy::AlignContent::FLEX_START,
        ContentAlignment::FlexEnd => taffy::AlignContent::FLEX_END,
        ContentAlignment::Center => taffy::AlignContent::CENTER,
        ContentAlignment::Stretch => taffy::AlignContent::STRETCH,
        ContentAlignment::SpaceBetween => taffy::AlignContent::SPACE_BETWEEN,
        ContentAlignment::SpaceAround => taffy::AlignContent::SPACE_AROUND,
        ContentAlignment::SpaceEvenly => taffy::AlignContent::SPACE_EVENLY,
    }
}

fn item_alignment(value: ItemAlignment) -> taffy::AlignItems {
    match value {
        ItemAlignment::Normal => taffy::AlignItems::NORMAL,
        ItemAlignment::Stretch => taffy::AlignItems::STRETCH,
        ItemAlignment::Start => taffy::AlignItems::START,
        ItemAlignment::End => taffy::AlignItems::END,
        ItemAlignment::FlexStart => taffy::AlignItems::FLEX_START,
        ItemAlignment::FlexEnd => taffy::AlignItems::FLEX_END,
        ItemAlignment::Center => taffy::AlignItems::CENTER,
        ItemAlignment::Baseline => taffy::AlignItems::BASELINE,
    }
}

/// `auto` is `None`: the item takes the `align-items`/`justify-items` of its container.
fn self_alignment(value: SelfAlignment) -> Option<taffy::AlignSelf> {
    Some(match value {
        SelfAlignment::Auto => return None,
        SelfAlignment::Normal => taffy::AlignItems::NORMAL,
        SelfAlignment::Stretch => taffy::AlignItems::STRETCH,
        SelfAlignment::Start => taffy::AlignItems::START,
        SelfAlignment::End => taffy::AlignItems::END,
        SelfAlignment::FlexStart => taffy::AlignItems::FLEX_START,
        SelfAlignment::FlexEnd => taffy::AlignItems::FLEX_END,
        SelfAlignment::Center => taffy::AlignItems::CENTER,
        SelfAlignment::Baseline => taffy::AlignItems::BASELINE,
    })
}

fn track_size<T>(size: TrackSize) -> T
where
    T: TaffyAuto
        + TaffyMinContent
        + TaffyMaxContent
        + taffy::style_helpers::FromLength
        + taffy::style_helpers::FromPercent
        + taffy::style_helpers::FromFr,
{
    match size {
        TrackSize::Auto => auto(),
        TrackSize::MinContent => min_content(),
        TrackSize::MaxContent => max_content(),
        TrackSize::Fraction(value) => fr(value),
        TrackSize::Length(LengthPercentage::PixelLength(px)) => length(px.0),
        TrackSize::Length(LengthPercentage::Percentage(p)) => percent(p.0),
        TrackSize::Length(LengthPercentage::Both { length: px, .. }) => length(px.0),
    }
}

fn track_list<T>(list: &TrackList) -> Vec<T>
where
    T: TaffyAuto
        + TaffyMinContent
        + TaffyMaxContent
        + taffy::style_helpers::FromLength
        + taffy::style_helpers::FromPercent
        + taffy::style_helpers::FromFr,
{
    list.0.iter().map(|&size| track_size(size)).collect()
}

fn grid_placement(value: GridPlacement) -> taffy::GridPlacement<String> {
    match value {
        GridPlacement::Auto => taffy::GridPlacement::Auto,
        GridPlacement::Line(index) => line(index),
        GridPlacement::Span(count) => span(count),
    }
}

/// Whether the box is taken out of the flow (`position: absolute`).
pub fn is_out_of_flow(style: &ComputedValues) -> bool {
    style.box_.position == PositionSpec::Absolute
}

/// The Taffy style of a box.
///
/// Inline-level boxes never reach Taffy as nodes of their own: inline content is measured as a
/// whole by the box it is in, and `inline-block` boxes are laid out as block containers of their
/// own and then placed on a line.
pub fn taffy_style(style: &ComputedValues) -> taffy::Style {
    let pos = &style.position;
    let box_ = &style.box_;
    let display = match box_.display {
        Display::None | Display::TableColumn | Display::TableColumnGroup => taffy::Display::None,
        Display::Block | Display::Inline | Display::InlineBlock => taffy::Display::Block,
        Display::Flex => taffy::Display::Flex,
        Display::Grid => taffy::Display::Grid,
        // A table is laid out as a grid (see `tree::HeliumTree::build_table`). The internal
        // table boxes are no Taffy nodes of their own, except for cells and captions, which are
        // block containers.
        Display::Table | Display::InlineTable => taffy::Display::Grid,
        Display::TableRowGroup
        | Display::TableHeaderGroup
        | Display::TableFooterGroup
        | Display::TableRow
        | Display::TableCell
        | Display::TableCaption => taffy::Display::Block,
    };
    let is_table = box_.display.is_table();
    let mut gap = Size {
        width: length_percentage(box_.column_gap),
        height: length_percentage(box_.row_gap),
    };
    if is_table {
        // https://www.w3.org/TR/CSS22/tables.html#separated-borders
        gap = Size {
            width: TLengthPercentage::length(style.text.border_spacing.horizontal.0),
            height: TLengthPercentage::length(style.text.border_spacing.vertical.0),
        };
    }
    // Tables are always laid out as `box-sizing: border-box` (their `width` includes the border
    // and padding). Everything else follows the `box-sizing` property.
    let box_sizing = if is_table {
        TBoxSizing::BorderBox
    } else {
        match box_.box_sizing {
            BoxSizing::ContentBox => TBoxSizing::ContentBox,
            BoxSizing::BorderBox => TBoxSizing::BorderBox,
        }
    };
    taffy::Style {
        display,
        flex_direction: match box_.flex_direction {
            FlexDirection::Row => taffy::FlexDirection::Row,
            FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
            FlexDirection::Column => taffy::FlexDirection::Column,
            FlexDirection::ColumnReverse => taffy::FlexDirection::ColumnReverse,
        },
        flex_wrap: match box_.flex_wrap {
            FlexWrap::Nowrap => taffy::FlexWrap::NoWrap,
            FlexWrap::Wrap => taffy::FlexWrap::Wrap,
            FlexWrap::WrapReverse => taffy::FlexWrap::WrapReverse,
        },
        flex_grow: box_.flex_grow,
        flex_shrink: box_.flex_shrink,
        flex_basis: dimension(box_.flex_basis),
        justify_content: content_alignment(box_.justify_content),
        align_content: content_alignment(box_.align_content),
        justify_items: item_alignment(box_.justify_items),
        align_items: item_alignment(box_.align_items),
        justify_self: self_alignment(box_.justify_self),
        align_self: self_alignment(box_.align_self),
        gap,
        // a table is as wide as its contents in block layout instead of filling its container
        item_is_table: is_table,
        grid_template_columns: track_list(&box_.grid_template_columns),
        grid_template_rows: track_list(&box_.grid_template_rows),
        grid_auto_columns: track_list(&box_.grid_auto_columns),
        grid_auto_rows: track_list(&box_.grid_auto_rows),
        grid_auto_flow: match box_.grid_auto_flow {
            GridAutoFlow::Row => taffy::GridAutoFlow::Row,
            GridAutoFlow::Column => taffy::GridAutoFlow::Column,
        },
        grid_column: taffy::Line {
            start: grid_placement(box_.grid_column_start),
            end: grid_placement(box_.grid_column_end),
        },
        grid_row: taffy::Line {
            start: grid_placement(box_.grid_row_start),
            end: grid_placement(box_.grid_row_end),
        },
        box_sizing,
        position: match style.box_.position {
            PositionSpec::Static => Position::Static,
            PositionSpec::Absolute => Position::Absolute,
        },
        inset: Rect {
            left: length_percentage_auto(pos.left),
            right: length_percentage_auto(pos.right),
            top: length_percentage_auto(pos.top),
            bottom: length_percentage_auto(pos.bottom),
        },
        size: Size {
            width: dimension(pos.width),
            height: dimension(pos.height),
        },
        // `auto` is the initial value of `min-*`, which Taffy treats as zero for block boxes, and
        // is used for `max-*: none` (no limit).
        min_size: Size {
            width: length_percentage_auto(pos.min_width),
            height: length_percentage_auto(pos.min_height),
        },
        max_size: Size {
            width: length_percentage_auto(pos.max_width),
            height: length_percentage_auto(pos.max_height),
        },
        margin: Rect {
            left: length_percentage_auto(pos.margin_left),
            right: length_percentage_auto(pos.margin_right),
            top: length_percentage_auto(pos.margin_top),
            bottom: length_percentage_auto(pos.margin_bottom),
        },
        padding: Rect {
            left: length_percentage(pos.padding_left),
            right: length_percentage(pos.padding_right),
            top: length_percentage(pos.padding_top),
            bottom: length_percentage(pos.padding_bottom),
        },
        border: Rect {
            left: TLengthPercentage::length(pos.border_left_width.0),
            right: TLengthPercentage::length(pos.border_right_width.0),
            top: TLengthPercentage::length(pos.border_top_width.0),
            bottom: TLengthPercentage::length(pos.border_bottom_width.0),
        },
        ..taffy::Style::DEFAULT
    }
}
