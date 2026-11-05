use super::components::{self, BlockLevelElement, LayoutEntity, MathNode, Position, RootNode};
use super::inline::InlineFormattingContext;
use super::margin_boxes::{self, MarginBoxes, ResolvedMarginBox};
use super::normal_formatting::BlockFormattingContext;
use super::{FontMap, text::Font};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::{MarginBoxKind, PropertyDeclarationBlock, Styles};
use crate::style::properties::computed::{
    ComputedValues, Display, LineHeight, PositionSpec, StyleBuilder, TextAlign, color_to_rgba,
};
use crate::style::properties::specified::{MathFamily, PropertyDeclaration, Text};
use crate::traverse::Traversal;
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{Attribute, QualName, local_name, ns};
use specs::{Builder, Entities, Entity, SystemData, World, WorldExt, WriteStorage};

use super::text::fallback::{FontResolver, register_font};
use components::TextRun;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
pub struct FlowConstructor {
    pub(crate) styles: Styles,
    stylesheet_base_dir: PathBuf,
    entity_constructor: EntityConstructor,
}

impl FlowConstructor {
    pub fn new(world: World, stylesheet_base_dir: PathBuf) -> Self {
        FlowConstructor {
            styles: Default::default(),
            stylesheet_base_dir,
            entity_constructor: EntityConstructor::new(world),
        }
    }

    /// Resolves the styles and fonts of the margin boxes (headers and footers) of the document.
    pub fn resolve_margin_boxes(&mut self) -> MarginBoxes {
        let mut resolve = |first: bool, left: bool| {
            let mut resolved = vec![];
            for kind in MarginBoxKind::ALL {
                let mut style_builder = StyleBuilder::new(Default::default());
                let mut content = None;
                let mut text_align = None;
                for rule in self.styles.rules_iter() {
                    let Some(selector) = rule.page_selector() else {
                        continue;
                    };
                    if !margin_boxes::selector_matches(selector, first, left) {
                        continue;
                    }
                    for margin_box in rule.margin_boxes.iter().filter(|b| b.kind == kind) {
                        for declaration in margin_box.properties.declarations() {
                            if let PropertyDeclaration::Text(Text::TextAlign(align)) = declaration {
                                text_align = Some(*align);
                            }
                            style_builder.add_declaration(declaration);
                        }
                        if margin_box.content.is_some() {
                            content = margin_box.content.clone();
                        }
                    }
                }
                let content = match content {
                    Some(content) if !content.is_empty() => content,
                    _ => continue,
                };
                let style = style_builder.build();
                let font = self.entity_constructor.match_font(&style);
                // page numbers are drawn later, but only consist of digits
                let static_text: String = content
                    .iter()
                    .filter_map(|item| match item {
                        crate::style::parse::ContentItem::String(string) => Some(string.as_str()),
                        _ => None,
                    })
                    .collect();
                self.entity_constructor
                    .ensure_coverage(&style, &font, &static_text);
                let font_name = font.postscript_name();
                resolved.push(ResolvedMarginBox {
                    kind,
                    content,
                    font_name,
                    font_size: style.font.font_size,
                    color: color_to_rgba(style.font.color),
                    text_align: text_align.unwrap_or(match kind {
                        MarginBoxKind::TopCenter | MarginBoxKind::BottomCenter => TextAlign::Center,
                        MarginBoxKind::TopRight | MarginBoxKind::BottomRight => TextAlign::Right,
                        MarginBoxKind::TopLeft | MarginBoxKind::BottomLeft => TextAlign::Left,
                    }),
                });
            }
            resolved
        };
        // the first page is a right page
        MarginBoxes {
            first: resolve(true, false),
            left: resolve(false, true),
            right: resolve(false, false),
        }
    }

    /// The world with the constructed boxes. It contains the characters that could not be
    /// rendered with any font as the `MissingGlyphs` resource.
    pub fn layout_context(mut self) -> super::components::LayoutContext {
        let missing = self.entity_constructor.font_resolver.take_missing_glyphs();
        self.entity_constructor.world.insert(missing);
        self.entity_constructor.world
    }

    fn read_stylesheet(&mut self, node: &DomRef) {
        if let ParsedData::Element(ElementData {
            name, attributes, ..
        }) = &node.html_data
        {
            if name == &QualName::new(None, ns!(html), local_name!("style")) {
                for child in TreeLinked::children(node) {
                    if let ParsedData::Text(css) = &child.html_data {
                        self.styles.parse_css(&css.borrow())
                    } else {
                        log::warn!("<style> element did not contain any style information.")
                    }
                }
            }
            if name == &QualName::new(None, ns!(html), local_name!("link")) {
                if let Some(Attribute { value, .. }) =
                    attributes
                        .borrow()
                        .get(&QualName::new(None, ns!(), local_name!("rel")))
                {
                    if value.as_ref() == "stylesheet" {
                        if let Some(Attribute { value, .. }) = attributes
                            .borrow()
                            .get(&QualName::new(None, ns!(), local_name!("href")))
                        {
                            self.styles
                                .add_stylesheet(value.as_ref(), &self.stylesheet_base_dir)
                        }
                    }
                }
            }
        }
    }
}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, node: DomRef) {
        self.read_stylesheet(&node);

        // TODO: Selector matching, Cascade
        match &node.clone().into_element_ref() {
            Some(element_ref) => {
                let parent_style = element_ref
                    .as_node()
                    .parent()
                    .and_then(|parent| NodeData::style(&parent));
                let parent_is_flex_or_grid = parent_style
                    .as_ref()
                    .is_some_and(|style| style.box_.display.is_flex_or_grid());
                let mut style_builder = StyleBuilder::new(parent_style.unwrap_or_default());
                // The cascade applies rules in ascending specificity order; the stable sort keeps
                // the source order of rules with equal specificity, so later rules still win.
                let mut matched_rules: Vec<_> = self
                    .styles
                    .rules_iter()
                    .filter_map(|rule| {
                        rule.matching_specificity(element_ref)
                            .map(|specificity| (specificity, rule))
                    })
                    .collect();
                matched_rules.sort_by_key(|&(specificity, _)| specificity);
                for (_, rule) in &matched_rules {
                    let properties = &rule.properties;
                    for declaration in properties.declarations() {
                        style_builder.add_declaration(declaration);
                    }
                }
                // declarations in the `style` attribute override all matched rules
                let inline_style = element_ref
                    .attributes
                    .borrow()
                    .get(&QualName::new(None, ns!(), local_name!("style")))
                    .map(|attr| PropertyDeclarationBlock::parse_inline(&attr.value));
                if let Some(block) = &inline_style {
                    for declaration in block.declarations() {
                        style_builder.add_declaration(declaration);
                    }
                }
                let mut style = style_builder.build();
                if parent_is_flex_or_grid {
                    style.blockify();
                }
                element_ref.as_node().set_style(style);
            }
            _ => {
                let parent_style = node.parent().and_then(|parent| NodeData::style(&parent));
                let style = StyleBuilder::new(parent_style.unwrap_or_default()).build();
                node.set_style(style);
            }
        }

        self.entity_constructor.build_entity(node)
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        match node.html_data() {
            ParsedData::Element(_) => self.entity_constructor.build_formatting_context(&node),
            _ => {}
        }
    }
}

/// The `colspan` and `rowspan` attributes of a table cell, if it has any.
/// https://html.spec.whatwg.org/multipage/tables.html#attributes-common-to-td-and-th-elements
fn table_cell_span(element_data: &ElementData) -> Option<components::TableCellSpan> {
    let attributes = element_data.attributes.borrow();
    let attribute = |name| {
        attributes
            .get(&QualName::new(None, ns!(), name))
            .and_then(|attr| attr.value.trim().parse::<u32>().ok())
    };
    let columns = attribute(local_name!("colspan"));
    let rows = attribute(local_name!("rowspan"));
    if columns.is_none() && rows.is_none() {
        return None;
    }
    Some(components::TableCellSpan {
        columns: columns.unwrap_or(1).clamp(1, 1000) as u16,
        rows: rows.unwrap_or(1).min(65534) as u16,
    })
}

pub trait AssignLayoutEntity {
    fn assign_layout_entity(&mut self, entity: LayoutEntity);
    fn layout_entity(&self) -> Option<LayoutEntity>;
}

impl AssignLayoutEntity for DomRef {
    fn assign_layout_entity(&mut self, entity: LayoutEntity) {
        self.layout_entity.set(Some(entity))
    }

    fn layout_entity(&self) -> Option<LayoutEntity> {
        self.layout_entity.get()
    }
}

pub trait Styled {
    fn style(&self) -> ComputedValues;
}

impl Styled for DomRef {
    fn style(&self) -> ComputedValues {
        NodeData::style(&self).expect("No style was assigned to DOM node.")
    }
}

pub trait HtmlNode: Sized {
    fn html_data(&self) -> &ParsedData;
}

impl HtmlNode for DomRef {
    fn html_data(&self) -> &ParsedData {
        &self.html_data
    }
}

pub trait IterChildren<'a>: Sized {
    type ChildrenIter: 'a + Iterator<Item = Self>;
    fn children(&'a self) -> Self::ChildrenIter;

    fn next_sibling(&self) -> Option<Self>;
    fn previous_sibling(&self) -> Option<Self>;
}

impl<'a> IterChildren<'a> for DomRef {
    type ChildrenIter = crate::tree::SiblingIter<DomRef>;
    fn children(&'a self) -> Self::ChildrenIter {
        TreeLinked::children(&self)
    }

    fn next_sibling(&self) -> Option<Self> {
        crate::tree::TreeLinked::next_sibling(&self)
    }

    fn previous_sibling(&self) -> Option<Self> {
        crate::tree::TreeLinked::previous_sibling(&self)
    }
}

pub trait AbstractNode<'a>: AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a> {}

impl<'a, T: 'a + AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a>> AbstractNode<'a> for T {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontSelector {
    families: Vec<crate::style::properties::computed::FontFamily>,
    weight: crate::style::properties::computed::FontWeight,
    style: crate::style::properties::computed::FontStyle,
}

impl FontSelector {
    fn from_style(style: &ComputedValues) -> Self {
        FontSelector {
            families: style.font.font_family.clone(),
            weight: style.font.font_weight.clone(),
            style: style.font.font_style.clone(),
        }
    }
}

pub struct EntityConstructor {
    font_match_cache: HashMap<FontSelector, String>,
    font_resolver: FontResolver,
    /// Whether the traversal is inside a `<math>` element.
    in_math: bool,
    pub world: World,
    trim_next_inline_whitespace: bool,
}

impl std::fmt::Debug for EntityConstructor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("EntityConstructor").finish()
    }
}

impl EntityConstructor {
    pub fn new(mut world: World) -> Self {
        world.register::<ComputedValues>();
        world.register::<InlineFormattingContext>();
        world.register::<BlockFormattingContext>();
        world.register::<Position>();
        world.register::<components::TextRun>();
        world.register::<components::ListMarker>();
        world.register::<components::Decoration>();
        world.register::<MathNode>();
        world.register::<components::TableCellSpan>();
        world.insert(FontMap::new());
        world.insert(RootNode(None));
        EntityConstructor {
            world,
            font_match_cache: Default::default(),
            font_resolver: Default::default(),
            in_math: false,
            trim_next_inline_whitespace: true,
        }
    }

    fn match_font(&mut self, style: &ComputedValues) -> Arc<Font> {
        let font_selector = FontSelector::from_style(&style);
        let mut font_map = self.world.write_resource::<FontMap>();
        let font_name = self
            .font_match_cache
            .entry(font_selector)
            .or_insert_with(|| {
                let font = crate::layout::text::font::match_font(&style);
                register_font(&mut font_map, font).postscript_name()
            });
        font_map[font_name].clone()
    }

    /// Adds fallback fonts to `font` for the characters of `text` it has no glyph for.
    fn ensure_coverage(&mut self, style: &ComputedValues, font: &Font, text: &str) {
        let mut font_map = self.world.write_resource::<FontMap>();
        self.font_resolver
            .ensure_coverage(style, font, text, &mut font_map);
    }

    // Called on top-down traversal. Takes a node and its style to create a corresponding
    // `LayoutEntity`, and assigns the entity to the node.
    pub fn build_entity<'a>(&mut self, mut node: impl AbstractNode<'a>) {
        let style = node.style();

        // Columns do not generate boxes.
        if style.box_.display == Display::None || style.box_.display.is_table_column() {
            return;
        }

        let entity = match node.html_data() {
            ParsedData::Element(element_data) => {
                if element_data.name == QualName::new(None, ns!(mathml), local_name!("math")) {
                    self.in_math = true;
                }
                if element_data.name == QualName::new(None, ns!(html), local_name!("br")) {
                    // treat <br> as whitespace
                    self.trim_next_inline_whitespace = true;
                }

                let is_list_item =
                    element_data.name == QualName::new(None, ns!(html), local_name!("li"));
                let ordinal = if is_list_item {
                    // the number of `li` siblings up to and including this one
                    let mut count = 1;
                    let mut sibling = node.previous_sibling();
                    while let Some(s) = sibling {
                        if let ParsedData::Element(data) = s.html_data() {
                            if data.name == QualName::new(None, ns!(html), local_name!("li")) {
                                count += 1;
                            }
                        }
                        sibling = s.previous_sibling();
                    }
                    count
                } else {
                    0
                };
                let marker_text = style
                    .font
                    .list_style_type
                    .marker_text(ordinal)
                    .filter(|_| is_list_item);
                let marker = marker_text.map(|text| {
                    let font = self.match_font(&style);
                    self.ensure_coverage(&style, &font, &text);
                    let font_name = font.postscript_name();
                    components::ListMarker {
                        text,
                        font_name,
                        font_size: style.font.font_size,
                        color: crate::style::properties::computed::color_to_rgba(style.font.color),
                    }
                });

                if style.box_.display.is_atomic_inline() {
                    // its contents are a block of their own
                    self.trim_next_inline_whitespace = true;
                }
                let span = (style.box_.display == Display::TableCell)
                    .then(|| table_cell_span(element_data))
                    .flatten();
                let entity = self.build_default_entity(style);
                if let Some(span) = span {
                    self.world
                        .write_storage::<components::TableCellSpan>()
                        .insert(entity.0, span)
                        .unwrap();
                }
                if let Some(marker) = marker {
                    let LayoutEntity(e) = entity;
                    self.world
                        .write_storage::<components::ListMarker>()
                        .insert(e, marker)
                        .unwrap();
                }
                entity
            }
            ParsedData::Document(_) => self.build_root_entity(),
            ParsedData::Text(string) => {
                // TODO: implement proper whitespace algorithm

                let mut text = string.borrow_mut();
                if style.text.white_space == crate::style::properties::specified::WhiteSpace::Pre {
                    *text = text
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        .replace('\t', "    ");
                } else {
                    match node.previous_sibling() {
                        Some(node)
                            if matches!(
                                node.style().box_.display,
                                Display::Inline | Display::InlineBlock | Display::InlineTable
                            ) => {}
                        _ => self.trim_next_inline_whitespace = true,
                    };

                    super::text::whitespace::transform_text(
                        &mut text,
                        self.trim_next_inline_whitespace,
                    );

                    if &*text != text.trim_end() {
                        // this text node ends with whitespace
                        self.trim_next_inline_whitespace = true
                    } else {
                        self.trim_next_inline_whitespace = false
                    }
                }

                if text.is_empty() {
                    return;
                } else {
                    self.build_text_entity(style, &*text)
                }
            }
            _ => self.build_default_entity(style),
        };

        node.assign_layout_entity(entity);
    }

    // called on top-down traversal
    fn build_root_entity(&mut self) -> LayoutEntity {
        self.build_default_entity(Default::default())
    }

    // called on top-down traversal
    fn build_default_entity(&mut self, style: ComputedValues) -> LayoutEntity {
        self.world.create_entity().with(style).build().into()
    }

    fn build_text_entity(&mut self, style: ComputedValues, text: &str) -> LayoutEntity {
        let font = self.match_font(&style);
        let font_name = font.postscript_name();
        let mut visible_text = String::with_capacity(text.len());
        let mut manual_hyphen_points = Vec::new();
        for character in text.chars() {
            if character == '\u{00ad}' {
                manual_hyphen_points.push(visible_text.len());
            } else {
                visible_text.push(character);
            }
        }
        self.ensure_coverage(&style, &font, &visible_text);
        // a hyphen is inserted when a word is hyphenated
        self.ensure_coverage(&style, &font, "-");

        let font_size = style.font.font_size;
        let font_height = font.scale_to_px(font.font_height(), font_size);
        let line_height = match style.font.line_height {
            LineHeight::Normal => font_height * 1.2,
            LineHeight::LengthPercentage(lp) => lp.absolute(style.font.font_size),
        };
        let line_gap = line_height - font_height;
        let baseline = font.scale_to_px(font.ascent(), font_size) + line_gap / 2.0;

        let color = crate::style::properties::computed::color_to_rgba(style.font.color);
        let decoration = style.font.text_decoration;
        let hyphens = style.text.hyphens;
        let entity = self
            .world
            .create_entity()
            .with(style)
            .with(components::TextRun {
                font_name,
                text: visible_text,
                font_size,
                line_height,
                baseline,
                color,
                decoration,
                hyphens,
                manual_hyphen_points,
            })
            .build();
        LayoutEntity(entity)
    }

    // Called on bottom-up traversal. Traverses the node's children and decides (based on their
    // computed styles) whether to build a block formatting context or an inline formatting
    // context.
    pub fn build_formatting_context<'a, Node: AbstractNode<'a>>(&mut self, node: &'a Node) {
        if node.layout_entity().is_none() {
            return;
        }

        if self.in_math {
            self.build_math_node(node);
            return;
        }

        if node.style().box_.display.is_atomic_inline() {
            // an inline-block box is content, so the whitespace after it must be kept
            self.trim_next_inline_whitespace = false;
        }

        let is_display_inline = |node: &Node| match node.style().box_.display {
            Display::Inline => true,
            _ => false,
        };

        if is_display_inline(node) {
            self.build_inline_formatting_context(node)
        } else {
            self.build_block_formatting_context(node)
        }
    }

    fn build_anonymous_block(
        entities: &Entities,
        inline_formatting_contexts: &mut WriteStorage<InlineFormattingContext>,
        computed_values: &mut WriteStorage<ComputedValues>,
        elements: &[Entity],
        style: &ComputedValues,
    ) -> Entity {
        // build anonymous block element
        entities
            .build_entity()
            .with(
                InlineFormattingContext::new(elements.to_owned(), vec![], style),
                inline_formatting_contexts,
            )
            .with(style.anonymous_block(), computed_values)
            .build()
    }

    fn build_blocks(
        elem_iter: impl Iterator<Item = Entity>,
        inline_level_elements: &mut Vec<Entity>,
        block_level_elements: &mut Vec<BlockLevelElement>,
        style: &ComputedValues,
        entities: &Entities,
        computed_values: &mut WriteStorage<ComputedValues>,
        inline_formatting_contexts: &mut WriteStorage<InlineFormattingContext>,
        block_formatting_contexts: &mut WriteStorage<BlockFormattingContext>,
    ) {
        let push_block =
            |ble: &mut Vec<BlockLevelElement>,
             entity,
             block_formatting_contexts: &mut WriteStorage<BlockFormattingContext>,
             inline_formatting_contexts: &WriteStorage<InlineFormattingContext>| {
                if let Some(child_bfc) = block_formatting_contexts.remove(entity) {
                    ble.extend_from_slice(&child_bfc.elements);
                } else {
                    ble.push(BlockLevelElement::ContainerStart(entity));
                    if let Some(ifc) = inline_formatting_contexts.get(entity) {
                        ble.extend(
                            ifc.absolutes
                                .iter()
                                .map(|&a| BlockLevelElement::Absolute(a)),
                        );
                    }
                    ble.push(BlockLevelElement::Leaf(entity));
                    ble.push(BlockLevelElement::ContainerEnd(entity));
                }
            };

        for elem in elem_iter {
            let child_style = computed_values.get(elem);
            let display = child_style
                .map(|s| s.box_.display)
                .unwrap_or(Display::Inline);
            let position = child_style.map(|s| s.box_.position).unwrap_or_default();

            if position == PositionSpec::Absolute {
                block_level_elements.push(BlockLevelElement::Absolute(elem));
                continue;
            }

            if display.is_atomic_inline() {
                // an atomic inline-level box: its contents stay in its own formatting context
                inline_level_elements.push(elem);
                continue;
            }

            if display.is_block_level() {
                if !inline_level_elements.is_empty() {
                    push_block(
                        block_level_elements,
                        Self::build_anonymous_block(
                            entities,
                            inline_formatting_contexts,
                            computed_values,
                            inline_level_elements,
                            style,
                        ),
                        block_formatting_contexts,
                        inline_formatting_contexts,
                    );
                    inline_level_elements.clear();
                }
                push_block(
                    block_level_elements,
                    elem,
                    block_formatting_contexts,
                    inline_formatting_contexts,
                );
            } else {
                if let Some(InlineFormattingContext {
                    elements,
                    absolutes,
                    ..
                }) = inline_formatting_contexts.remove(elem)
                {
                    block_level_elements
                        .extend(absolutes.into_iter().map(BlockLevelElement::Absolute));
                    Self::build_blocks(
                        elements.iter().cloned(),
                        inline_level_elements,
                        block_level_elements,
                        style,
                        entities,
                        computed_values,
                        inline_formatting_contexts,
                        block_formatting_contexts,
                    );
                } else {
                    inline_level_elements.push(elem);
                }
            }
        }
    }

    // called on bottom-up traversal
    fn build_block_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let entities = self.world.entities();
        let mut block_formatting_contexts = self.world.write_storage::<BlockFormattingContext>();
        let mut inline_formatting_contexts = self.world.write_storage::<InlineFormattingContext>();
        let mut computed_values = self.world.write_storage::<ComputedValues>();

        let LayoutEntity(entity) = node.layout_entity().unwrap();

        let style = computed_values.get(entity).unwrap().clone();

        let mut block_level_elements = vec![];
        let mut inline_level_elements = vec![];

        let entity_iter = node
            .children()
            .filter_map(|child| child.layout_entity().map(|LayoutEntity(ent)| ent));

        Self::build_blocks(
            entity_iter,
            &mut inline_level_elements,
            &mut block_level_elements,
            &style,
            &entities,
            &mut computed_values,
            &mut inline_formatting_contexts,
            &mut block_formatting_contexts,
        );

        let has_block_level_elements = block_level_elements
            .iter()
            .any(|elem| !matches!(elem, BlockLevelElement::Absolute(_)));
        if !has_block_level_elements {
            // no block-level elements were found, so we create a inline_formatting_context
            let absolutes = block_level_elements
                .iter()
                .map(|elem| elem.entity())
                .collect();
            inline_formatting_contexts
                .insert(
                    entity,
                    InlineFormattingContext::new(inline_level_elements, absolutes, &style),
                )
                .unwrap();
            return;
        }

        // push last anonymous block, should there be one
        if !inline_level_elements.is_empty() {
            let entity = Self::build_anonymous_block(
                &entities,
                &mut inline_formatting_contexts,
                &mut computed_values,
                &inline_level_elements,
                &style,
            );
            block_level_elements.extend_from_slice(&[
                BlockLevelElement::ContainerStart(entity),
                BlockLevelElement::Leaf(entity),
                BlockLevelElement::ContainerEnd(entity),
            ]);
        }

        block_level_elements.insert(0, BlockLevelElement::ContainerStart(entity));
        block_level_elements.push(BlockLevelElement::ContainerEnd(entity));

        block_formatting_contexts
            .insert(
                entity,
                BlockFormattingContext {
                    elements: block_level_elements,
                },
            )
            .unwrap();

        let mut root_node = self.world.write_resource::<RootNode>();
        *root_node = RootNode(Some(entity));
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let mut inline_formatting_context =
            InlineFormattingContext::new(vec![], vec![], &node.style());
        for child in node.children() {
            let child_entity = match child.layout_entity() {
                Some(LayoutEntity(entity)) => entity,
                None => continue,
            };

            let child_style = child.style();
            if child_style.box_.position == PositionSpec::Absolute {
                inline_formatting_context.absolutes.push(child_entity);
                continue;
            }

            if child_style.box_.display.is_atomic_inline() {
                inline_formatting_context.elements.push(child_entity);
                continue;
            }

            // merge child's InlineFormattingContext into the current context
            if let Some(InlineFormattingContext {
                elements,
                absolutes,
                ..
            }) = self.world.write_storage().remove(child_entity)
            {
                inline_formatting_context
                    .elements
                    .extend_from_slice(&elements);
                inline_formatting_context.absolutes.extend(absolutes);
                continue;
            }

            inline_formatting_context.elements.push(child_entity);
        }

        if inline_formatting_context.elements.is_empty() {
            // insert strut
            inline_formatting_context
                .elements
                .push(self.build_text_entity(node.style(), "").0);
        }

        // `unicode-bidi` (e.g. from the `dir` attribute or `<bdo>`) is implemented by bidi
        // control characters around the contents of the element, which are zero-width text runs
        // of their own.
        let style = node.style();
        if let Some((open, close)) = style
            .box_
            .unicode_bidi
            .control_characters(style.text.direction)
        {
            let open = self.build_text_entity(style.clone(), open).0;
            let close = self.build_text_entity(style, close).0;
            inline_formatting_context.elements.insert(0, open);
            inline_formatting_context.elements.push(close);
        }

        let LayoutEntity(self_entity) = node.layout_entity().unwrap();
        self.world
            .write_storage()
            .insert(self_entity, inline_formatting_context)
            .unwrap();
    }

    // called on bottom-up traversal
    fn build_math_node<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        use math_render::host::{MathElement, NodeKind};
        use math_render::unicode_math::StringExtMathml;

        let ParsedData::Element(element_data) = node.html_data() else {
            return;
        };
        let LayoutEntity(entity) = node.layout_entity().unwrap();
        let style = node.style();
        let font_name = self.match_font(&style).postscript_name();

        let kind = if element_data.name.ns == ns!(mathml) {
            NodeKind::Math(MathElement::from_local_name(&element_data.name.local))
        } else {
            // e.g. HTML inside `<mtext>`
            NodeKind::Foreign
        };
        let is_token = matches!(kind, NodeKind::Math(element) if element.is_token());

        let attributes: HashMap<String, String> = element_data
            .attributes
            .borrow()
            .iter()
            .filter(|(name, _)| name.ns == ns!())
            .map(|(name, attr)| (name.local.to_string(), attr.value.to_string()))
            .collect();

        let math_nodes = WriteStorage::<MathNode>::fetch(&self.world);
        let text_runs = WriteStorage::<TextRun>::fetch(&self.world);
        let mut children = Vec::new();
        let mut raw_text = String::new();
        for child in node.children() {
            let Some(LayoutEntity(child_entity)) = child.layout_entity() else {
                continue;
            };
            if let Some(text_run) = text_runs.get(child_entity) {
                raw_text.push_str(&text_run.text);
            } else if math_nodes.contains(child_entity) {
                children.push(child_entity);
            }
        }

        let text = is_token.then(|| {
            let is_mi = kind == NodeKind::Math(MathElement::Mi);
            let is_mo = kind == NodeKind::Math(MathElement::Mo);
            // By default, an `mi` is only italic if it consists of a single character, longer
            // identifiers (e.g. `sin`) are upright.
            let variant = if is_mi
                && !attributes.contains_key("mathvariant")
                && raw_text.chars().count() != 1
            {
                MathFamily::Normal
            } else {
                style.math.math_variant
            };
            let text = raw_text.adapt_to_family(Some(variant.into()));
            text.replace_anomalous_characters(is_mo)
        });
        drop((math_nodes, text_runs));

        self.world
            .write_storage::<MathNode>()
            .insert(
                entity,
                MathNode {
                    kind,
                    children,
                    text,
                    attributes,
                    font_name,
                },
            )
            .unwrap();

        if element_data.name == QualName::new(None, ns!(mathml), local_name!("math")) {
            self.in_math = false;
        }
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    use crate::simple_dom::{Quantity::Px, div, p, span, text};

    use crate::style::properties::computed::{ComputedValues, GenericSize};

    #[test]
    fn test_block_flow_creation_from_dom_tree() {
        use BlockLevelElement::*;

        let html = div(vec![div(vec![])]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerEnd(html.entity()),
            ]
        )
    }

    #[test]
    fn test_inline_flow_creation_from_dom_tree() {
        let html = div(vec![text("a"), text("b")]);
        let world = html.construct();

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html.get(&inline_fmt);

        assert_eq!(elements, &[html[0].entity(), html[1].entity()]);
    }

    #[test]
    fn test_nested_inline_flow_creation_from_dom_tree() {
        use BlockLevelElement::*;

        let html = div(vec![div(vec![text("a"), text("b")])]);
        let world = html.construct();

        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);
        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerEnd(html.entity()),
            ]
        );

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html[0].get(&inline_fmt);
        assert_eq!(elements, &[html[0][0].entity(), html[0][1].entity()]);
    }

    #[test]
    fn test_nested_inlines() {
        let html = div(vec![
            span(vec![span(vec![text("Abc")]), span(vec![text("efg")])]),
            span(vec![text("hij")]),
        ]);
        let world = html.construct();

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html.get(&inline_fmt);
        assert_eq!(
            elements,
            &[
                html[0][0][0].entity(),
                html[0][1][0].entity(),
                html[1][0].entity()
            ]
        )
    }

    #[test]
    fn test_text_run_creation_from_text_node() {
        let html = text("Hello World!");
        let world = html.construct();
        let text_run = world.read_storage();
        let components::TextRun { text, .. } = html.get(&text_run);
        assert_eq!(text, "Hello World!");
    }

    #[test]
    fn test_anonymous_block_box_for_text_node_creation() {
        use BlockLevelElement::*;

        // <div>
        //   <div></div>
        //   Hello World
        // </div>
        let html = div(vec![div(vec![]), text("Hello World!")]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                // these are the three anonymous node
                ContainerStart(elements[4].entity()),
                Leaf(elements[5].entity()),
                ContainerEnd(elements[6].entity()),
                // end of anonymous nodes
                ContainerEnd(html.entity()),
            ]
        );

        if let Leaf(entity) = elements[5] {
            let inline_fmt = world.read_storage();
            let InlineFormattingContext { elements, .. } = inline_fmt.get(entity).unwrap();
            assert_eq!(elements, &[html[1].entity()]);
        } else {
            panic!()
        }
    }

    #[test]
    fn test_nested_block_level_elements_creation() {
        use BlockLevelElement::*;

        // <div>
        //   <p>Something</p>
        //   <div>
        //      <p>something nested</p>
        //   </div>
        //   <p>something else</p>
        // </div>
        let html = div(vec![
            p(vec![text("Something")]),
            div(vec![p(vec![text("something nested")])]),
            p(vec![text("something else")]),
        ]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerStart(html[1].entity()),
                ContainerStart(html[1]["p"].entity()),
                Leaf(html[1]["p"].entity()),
                ContainerEnd(html[1]["p"].entity()),
                ContainerEnd(html[1].entity()),
                ContainerStart(html[2].entity()),
                Leaf(html[2].entity()),
                ContainerEnd(html[2].entity()),
                ContainerEnd(html.entity()),
            ]
        );

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html[1]["p"].get(&inline_fmt);

        assert_eq!(elements, &[html[1]["p"][0].entity()])
    }

    #[test]
    fn test_if_styled_node_has_style_component() {
        let html = div(vec![]).margin_top(Px(10.0));
        let world = html.construct();
        let styles = world.read_storage();
        let style: &ComputedValues = html.get(&styles);

        assert_eq!(style.position.margin_top, GenericSize::from(Px(10.0)))
    }
}
