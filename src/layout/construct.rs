use super::components::{self, BlockLevelElement, LayoutEntity, MathNode, Position, RootNode};
use super::inline::InlineFormattingContext;
use super::margin_boxes::{MarginBoxes, ResolvedMarginBox};
use super::pages::{self, PageContext};
use super::normal_formatting::BlockFormattingContext;
use super::{FontMap, text::Font};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::{MarginBoxKind, PropertyDeclarationBlock, Styles};
use crate::style::properties::computed::{
    ComputedValues, Display, GenericSize, LineHeight, PixelLength as Px, PositionSpec, StyleBuilder,
    TextAlign, color_to_rgba,
};
use crate::style::properties::specified::{MathFamily, PropertyDeclaration, Text};
use crate::traverse::{Traversal, Traverse};
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{Attribute, QualName, local_name, ns};
use specs::{Builder, Entities, Entity, SystemData, World, WorldExt, WriteStorage};

use super::text::fallback::{FontResolver, TextFonts, register_font};
use components::TextRun;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The content of a `foreignObject` element of an SVG image: laid out on its own, as a block of a
/// given size, because the SVG decides where it goes.
#[derive(Debug, Clone)]
pub struct IslandSpec {
    /// The `foreignObject` element, whose children are the content.
    pub node: DomRef,
    /// The size of the element in pixels, known when the SVG image is read.
    pub width: f32,
    pub height: f32,
}

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
pub struct FlowConstructor {
    pub(crate) styles: Styles,
    stylesheet_base_dir: PathBuf,
    entity_constructor: EntityConstructor,
    /// The `<svg>` element whose contents are being traversed: they are not laid out.
    inside_svg: Option<DomRef>,
    /// The outermost `<svg>` elements of the document, found when the first one is needed.
    svg_roots: Option<Vec<DomRef>>,
    /// The `foreignObject` elements of the SVG images, in the order of their handles.
    pub islands: Vec<IslandSpec>,
    /// The `foreignObject` element that is laid out as the root of the document, with its size.
    island_root: Option<(DomRef, f32, f32)>,
}

impl FlowConstructor {
    /// Records the `vertical-align` of an inline-level element together with the metrics of its
    /// parent's font, which `sub`, `super`, `middle` and `text-top`/`text-bottom` refer to.
    fn record_vertical_align(&mut self, node: &DomRef) {
        use crate::style::properties::computed::VerticalAlign;
        let Some(style) = NodeData::style(node) else { return };
        let display = style.box_.display;
        if style.box_.vertical_align == VerticalAlign::Baseline
            || !(display == Display::Inline || display.is_atomic_inline())
        {
            return;
        }
        let (Some(LayoutEntity(entity)), Some(parent)) = (node.layout_entity(), node.parent())
        else {
            return;
        };
        let Some(parent_style) = NodeData::style(&parent) else { return };
        // (an image is made an atomic inline-level box in its entity only)
        let atomic = self
            .entity_constructor
            .world
            .read_storage::<ComputedValues>()
            .get(entity)
            .is_some_and(|style| style.box_.display.is_atomic_inline());
        let step = self.entity_constructor.align_step(&style, &parent_style, atomic);
        let _ = self
            .entity_constructor
            .world
            .write_storage::<components::ElementAlign>()
            .insert(entity, components::ElementAlign(step));
    }

    pub fn new(world: World, stylesheet_base_dir: PathBuf) -> Self {
        Self::with_styles(world, stylesheet_base_dir, Default::default())
    }

    /// A constructor that uses the given style sheets (which include the ones of the user agent).
    fn with_styles(world: World, stylesheet_base_dir: PathBuf, styles: Styles) -> Self {
        FlowConstructor {
            styles,
            stylesheet_base_dir: stylesheet_base_dir.clone(),
            entity_constructor: EntityConstructor::new(world, stylesheet_base_dir.clone()),
            inside_svg: None,
            svg_roots: None,
            islands: vec![],
            island_root: None,
        }
    }

    /// A constructor for the content of a `foreignObject`: it works on the style sheets of the
    /// document that the element is in and on its fonts. `islands` are the ones found so far.
    pub fn for_island(
        world: World,
        stylesheet_base_dir: PathBuf,
        styles: Styles,
        fonts: FontMap,
        island: (DomRef, f32, f32),
        islands: Vec<IslandSpec>,
    ) -> Self {
        let mut constructor = FlowConstructor::with_styles(world, stylesheet_base_dir, styles);
        *constructor.entity_constructor.world.write_resource::<FontMap>() = fonts;
        constructor.use_document_fonts();
        constructor.island_root = Some(island);
        constructor.islands = islands;
        constructor
    }

    /// Uses the fonts of the style sheets that were read (their `@font-face` rules).
    pub fn use_document_fonts(&mut self) {
        let fonts = self.styles.font_context();
        self.entity_constructor.set_fonts(fonts);
    }

    /// Takes the style sheets and the `foreignObject` elements out of the constructor.
    pub fn take_shared(&mut self) -> (Styles, Vec<IslandSpec>) {
        (std::mem::replace(&mut self.styles, Styles::empty()), std::mem::take(&mut self.islands))
    }

    /// The image of an `<svg>` element: its contents are drawn as vector graphics.
    fn svg_image(
        &mut self,
        node: &DomRef,
        style: &ComputedValues,
    ) -> Option<crate::image::LoadedImage> {
        let roots = self
            .svg_roots
            .get_or_insert_with(|| crate::svg_dom::svg_roots(node))
            .clone();
        let family = style
            .font
            .font_family
            .iter()
            .map(|family| match family {
                crate::style::properties::specified::FontFamily::SansSerif => "sans-serif".to_owned(),
                crate::style::properties::specified::FontFamily::Serif => "serif".to_owned(),
                crate::style::properties::specified::FontFamily::Monospace => "monospace".to_owned(),
                crate::style::properties::specified::FontFamily::Named(name) => {
                    format!("\"{}\"", name.replace('"', ""))
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let color = color_to_rgba(style.font.color);
        let inherited = crate::svg_dom::Inherited {
            color: Some(format!("rgb({}, {}, {})", color.red, color.green, color.blue)),
            font_family: Some(family),
            font_size: Some(format!("{}px", style.font.font_size.0)),
        };
        let document =
            crate::svg_dom::document(node, &roots, &self.styles, &inherited, &mut self.islands);
        match crate::image::svg_from_dom(
            &document,
            &self.stylesheet_base_dir,
            &self.entity_constructor.fonts,
            style.font.font_size.0,
        ) {
            Ok(image) => {
                if let crate::image::LoadedImage::Svg(svg) = &image {
                    foreign_sizes(svg.tree.root(), &mut self.islands);
                }
                Some(image)
            }
            Err(error) => {
                log::warn!("cannot draw an <svg> element: {}", error);
                None
            }
        }
    }

    /// Computes the style of the node from the rules of the style sheets and the style of its
    /// parent.
    fn compute_style(&mut self, node: &DomRef) {
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
                // the root element is the one without a parent element (its parent is the document)
                let is_root = element_ref
                    .as_node()
                    .parent()
                    .map_or(true, |parent| parent.into_element_ref().is_none());
                if is_root {
                    style_builder = style_builder.root();
                }
                // The cascade applies the declarations in ascending order of priority: those of
                // the user agent, of the author's rules, and of the `style` attribute, then the
                // `!important` ones of the author and finally the ones of the user agent. Rules
                // of one origin go by specificity; the stable sort keeps the source order of rules
                // with equal specificity, so later rules still win.
                let mut matched_rules: Vec<_> = self
                    .styles
                    .rules_iter()
                    .enumerate()
                    .filter_map(|(index, rule)| {
                        rule.matching_specificity(element_ref)
                            .map(|specificity| (self.styles.is_user_agent_rule(index), specificity, rule))
                    })
                    .collect();
                matched_rules.sort_by_key(|&(user_agent, specificity, _)| (!user_agent, specificity));
                let inline_style = element_ref
                    .attributes
                    .borrow()
                    .get(&QualName::new(None, ns!(), local_name!("style")))
                    .map(|attr| {
                        let dir = &self.stylesheet_base_dir;
                        crate::style::url_context::with(dir, dir, || PropertyDeclarationBlock::parse_inline(&attr.value))
                    });
                let mut cascade = vec![];
                for (user_agent, _, rule) in &matched_rules {
                    if *user_agent {
                        cascade.extend(rule.properties.normal_declarations());
                    }
                }
                // the `width` and `height` attributes of an image are presentational hints: they
                // rank below all the rules of the author
                let size_hint = image_size_hint(element_ref).or_else(|| table_hint(node, element_ref));
                if let Some(block) = &size_hint {
                    cascade.extend(block.normal_declarations());
                }
                for (user_agent, _, rule) in &matched_rules {
                    if !*user_agent {
                        cascade.extend(rule.properties.normal_declarations());
                    }
                }
                // declarations in the `style` attribute override all matched rules
                if let Some(block) = &inline_style {
                    cascade.extend(block.normal_declarations());
                }
                for (user_agent, _, rule) in &matched_rules {
                    if !*user_agent {
                        cascade.extend(rule.properties.important_declarations());
                    }
                }
                if let Some(block) = &inline_style {
                    cascade.extend(block.important_declarations());
                }
                for (user_agent, _, rule) in &matched_rules {
                    if *user_agent {
                        cascade.extend(rule.properties.important_declarations());
                    }
                }
                style_builder.apply_cascade(cascade.iter().copied());
                let mut style = style_builder.build();
                if parent_is_flex_or_grid {
                    style.blockify();
                }
                // the content of a `foreignObject` is a block of the size of the element
                if let Some((root, width, height)) = &self.island_root {
                    if root == node {
                        std::sync::Arc::make_mut(&mut style.box_).display = Display::Block;
                        let position = std::sync::Arc::make_mut(&mut style.position);
                        position.width = Px(*width).into();
                        position.height = Px(*height).into();
                    }
                }
                // the language of the element, inherited from the parent unless it has its own
                // (`xml:lang` wins over `lang`)
                let lang = {
                    let attributes = element_ref.attributes.borrow();
                    attributes
                        .get(&QualName::new(None, ns!(xml), local_name!("lang")))
                        .or_else(|| attributes.get(&QualName::new(None, ns!(), local_name!("lang"))))
                        .map(|attr| crate::style::properties::computed::Language::from_tag(&attr.value))
                };
                if let Some(lang) = lang {
                    std::sync::Arc::make_mut(&mut style.font).lang = lang;
                }
                element_ref.as_node().set_style(style);
            }
            _ => {
                let parent_style = node.parent().and_then(|parent| NodeData::style(&parent));
                let style = StyleBuilder::new(parent_style.unwrap_or_default()).build();
                node.set_style(style);
            }
        }

    }

    /// Computes the style of the node, and before that the one of its ancestors, if they do not
    /// have one yet.
    fn ensure_style(&mut self, node: &DomRef) {
        if NodeData::style(node).is_some() {
            return;
        }
        if let Some(parent) = node.parent() {
            self.ensure_style(&parent);
        }
        self.compute_style(node);
    }

    /// Lays out the content of a `foreignObject`: the element is the root of the document.
    pub fn construct_island(&mut self) {
        let Some((root, _, _)) = self.island_root.clone() else {
            return;
        };
        self.ensure_style(&root);
        // the style of the root was computed from the ancestors: do it again with the size
        self.compute_style(&root);
        root.traverse(self);
        // the root is the element, also when it has no block-level content: the formatting
        // context that was built last is then one of an element inside of it
        if let Some(LayoutEntity(entity)) = root.layout_entity.get() {
            *self.entity_constructor.world.write_resource::<RootNode>() = RootNode(Some(entity));
        }
    }

    /// Gives a document without a box for its root element (`html { display: none }`) an empty
    /// root, so that it is a blank page.
    pub fn ensure_root(&mut self) {
        let RootNode(root) = *self.entity_constructor.world.read_resource::<RootNode>();
        if root.is_some() {
            return;
        }
        let LayoutEntity(entity) = self.entity_constructor.build_default_entity(Default::default());
        self.entity_constructor
            .world
            .write_storage::<BlockFormattingContext>()
            .insert(
                entity,
                BlockFormattingContext {
                    elements: vec![
                        BlockLevelElement::ContainerStart(entity),
                        BlockLevelElement::ContainerEnd(entity),
                    ],
                },
            )
            .unwrap();
        *self.entity_constructor.world.write_resource::<RootNode>() = RootNode(Some(entity));
    }

    /// Whether the document is set from right to left: the `direction` of the root element.
    pub fn is_rtl(&self) -> bool {
        let RootNode(root) = *self.entity_constructor.world.read_resource::<RootNode>();
        let styles = self.entity_constructor.world.read_storage::<ComputedValues>();
        root.and_then(|root| styles.get(root))
            .is_some_and(|style| style.text.direction == crate::style::properties::specified::Direction::Rtl)
    }

    /// Resolves the styles and fonts of the margin boxes (headers and footers) of the document.
    pub fn resolve_margin_boxes(&mut self) -> MarginBoxes {
        let names = pages::selected_names(&self.styles);
        let rtl = self.is_rtl();
        let mut resolve = |page: &PageContext| {
            let mut resolved = vec![];
            for kind in MarginBoxKind::ALL {
                let mut style_builder = StyleBuilder::new(Default::default());
                let mut content = None;
                let mut text_align = None;
                for rule in pages::matching_rules(&self.styles, page) {
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
                // page numbers are drawn later, but only consist of digits
                let static_text: String = content
                    .iter()
                    .filter_map(|item| match item {
                        crate::style::parse::ContentItem::String(string) => Some(string.as_str()),
                        _ => None,
                    })
                    .collect();
                // the text of the margin box is put together later (with the number of the page):
                // it takes the fonts of its clusters
                let (_, mut face) = self.entity_constructor.text_face(&style, &static_text);
                face.fallback = face.fallback.for_other_text();
                resolved.push(ResolvedMarginBox {
                    kind,
                    content,
                    face,
                    text_transform: style.text.text_transform,
                    lang: style.font.lang.clone(),
                    text_align: text_align.unwrap_or(match kind {
                        MarginBoxKind::TopCenter | MarginBoxKind::BottomCenter => TextAlign::Center,
                        MarginBoxKind::TopRight | MarginBoxKind::BottomRight => TextAlign::Right,
                        MarginBoxKind::TopLeft | MarginBoxKind::BottomLeft => TextAlign::Left,
                    }),
                });
            }
            resolved
        };
        let mut boxes = MarginBoxes {
            pages: HashMap::new(),
            names: names.clone(),
        };
        for page in pages::representative_pages(&names, rtl) {
            boxes.pages.insert(page.clone(), resolve(&page));
        }
        boxes
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
            // a style sheet for other media (`media="screen"`) is not read
            let for_this_media = attributes
                .borrow()
                .get(&QualName::new(None, ns!(), local_name!("media")))
                .map_or(true, |media| self.styles.media_matches(&media.value));
            if !for_this_media {
                return;
            }
            if name == &QualName::new(None, ns!(html), local_name!("style")) {
                for child in TreeLinked::children(node) {
                    if let ParsedData::Text(css) = &child.html_data {
                        // relative URLs in the style sheet are relative to the document
                        let dir = self.stylesheet_base_dir.clone();
                        crate::style::url_context::with(&dir, &dir, || {
                            self.styles.parse_css_in(&css.borrow(), &dir)
                        })
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

/// Reads the style sheets of the document before any style is computed: the rules of a `<style>`
/// element in the head apply to the root element, which comes before it.
pub struct StylesheetReader<'a>(pub &'a mut FlowConstructor);

impl Traversal<DomRef> for StylesheetReader<'_> {
    fn preorder_traverse(&mut self, node: DomRef) {
        self.0.read_stylesheet(&node);
    }

    fn postorder_traverse(&mut self, _node: DomRef) {}
}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, node: DomRef) {
        if self.inside_svg.is_some() {
            // the elements of an `<svg>` are drawn by the SVG code, not laid out
            return;
        }
        self.compute_style(&node);

        if crate::svg_dom::is_outermost_svg(&node) {
            if let Some(style) = NodeData::style(&node) {
                if style.box_.display != Display::None {
                    self.entity_constructor.pending_svg = self.svg_image(&node, &style);
                }
            }
            self.inside_svg = Some(node.clone());
        }
        let scoped = node.clone();
        self.entity_constructor.build_entity(node);
        self.entity_constructor.record_transform_scope(&scoped);
        self.record_vertical_align(&scoped);
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        if let Some(svg) = &self.inside_svg {
            if *svg != node {
                return;
            }
            self.inside_svg = None;
        }
        match node.html_data() {
            ParsedData::Element(_) => self.entity_constructor.build_formatting_context(&node),
            _ => {}
        }
        // A table's columns do not generate boxes, so their widths are recorded here, where the
        // table's entity exists and its `<col>` children have their styles.
        let is_table = NodeData::style(&node)
            .map(|style| style.box_.display == Display::Table)
            .unwrap_or(false);
        let Some(LayoutEntity(entity)) = node.layout_entity() else {
            return;
        };
        if is_table {
            let columns = table_columns(&node);
            if !columns.columns.is_empty() {
                self.entity_constructor
                    .world
                    .write_storage::<components::TableColumns>()
                    .insert(entity, columns)
                    .unwrap();
            }
        }
    }
}

/// The number shown next to a list item: `<ol start>` sets the first number and `<li value>`
/// restarts the count at that item.
/// https://html.spec.whatwg.org/multipage/grouping-content.html#the-ol-element
fn list_item_ordinal<'a, N: AbstractNode<'a>>(node: &N) -> i64 {
    /// The value of an attribute, if it is a valid HTML integer.
    fn integer(element_data: &ElementData, name: html5ever::LocalName) -> Option<i64> {
        let attributes = element_data.attributes.borrow();
        let attribute = attributes.get(&QualName::new(None, ns!(), name))?;
        attribute.value.trim().parse::<i64>().ok()
    }
    fn is_li(data: &ElementData) -> bool {
        data.name == QualName::new(None, ns!(html), local_name!("li"))
    }

    // An `<ol>` numbers from its `start` attribute; a list item that is not in an ordered list
    // gets no number of its own (its marker does not use one).
    let start = match node.parent_node() {
        Some(parent) => match parent.html_data() {
            ParsedData::Element(data)
                if data.name == QualName::new(None, ns!(html), local_name!("ol")) =>
            {
                integer(data, local_name!("start")).unwrap_or(1)
            }
            _ => 1,
        },
        None => 1,
    };

    // Run the count over the preceding list items in document order.
    let mut preceding = vec![];
    let mut previous = node.previous_sibling();
    while let Some(sibling) = previous {
        previous = sibling.previous_sibling();
        preceding.push(sibling);
    }
    let mut ordinal = start;
    for sibling in preceding.into_iter().rev() {
        if let ParsedData::Element(data) = sibling.html_data() {
            if is_li(data) {
                ordinal = integer(data, local_name!("value")).unwrap_or(ordinal) + 1;
            }
        }
    }

    match node.html_data() {
        ParsedData::Element(data) => integer(data, local_name!("value")).unwrap_or(ordinal),
        _ => ordinal,
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

/// The text a replaced element shows: the `value` of an `<input>`, or the state of a checkbox or
/// radio button. Only `<input>` is handled; a renderer without widgets has nothing else to show.
fn replaced_element_text<'a>(node: &'a impl AbstractNode<'a>) -> Option<String> {
    let ParsedData::Element(element_data) = node.html_data() else {
        return None;
    };
    if element_data.name != QualName::new(None, ns!(html), local_name!("input")) {
        return None;
    }

    let attributes = element_data.attributes.borrow();
    let has = |name| {
        attributes
            .get(&QualName::new(None, ns!(), name))
            .is_some()
    };
    let value = || {
        attributes
            .get(&QualName::new(None, ns!(), local_name!("value")))
            .map(|attribute| attribute.value.to_string())
    };
    let kind = attributes
        .get(&QualName::new(None, ns!(), local_name!("type")))
        .map(|attribute| attribute.value.to_ascii_lowercase())
        .unwrap_or_else(|| "text".into());

    Some(match kind.as_str() {
        // U+2610 and U+2611, the glyphs a font fallback is chosen for
        "checkbox" | "radio" => {
            if has(local_name!("checked")) {
                "\u{2611}".into()
            } else {
                "\u{2610}".into()
            }
        }
        _ => value().unwrap_or_default(),
    })
}

/// The `<col>` and `<colgroup>` children of a table, in column order.
/// https://html.spec.whatwg.org/multipage/tables.html#the-colgroup-element
fn table_columns(table: &DomRef) -> components::TableColumns {
    /// The `span` attribute, clamped to the range HTML allows.
    fn span(element_data: &ElementData) -> u16 {
        let attributes = element_data.attributes.borrow();
        attributes
            .get(&QualName::new(None, ns!(), local_name!("span")))
            .and_then(|attribute| attribute.value.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 1000) as u16
    }

    /// The `width` of a column element; `auto` and percentages of unknown size are skipped.
    fn width(style: Option<ComputedValues>) -> Option<GenericSize> {
        match style?.position.width {
            GenericSize::Auto => None,
            width => Some(width),
        }
    }

    fn collect(
        children: impl Iterator<Item = DomRef>,
        group_width: Option<GenericSize>,
        columns: &mut Vec<components::TableColumn>,
    ) {
        for child in children {
            let Some(style) = NodeData::style(&child) else {
                continue;
            };
            let display = style.box_.display;
            if !display.is_table_column() {
                continue;
            }
            let ParsedData::Element(element_data) = child.html_data() else {
                continue;
            };
            let span = span(element_data);
            let width = width(Some(style)).or(group_width);
            if display == Display::TableColumnGroup {
                // a group without `<col>` children covers its own columns
                let count = columns.len();
                collect(TreeLinked::children(&child), width, columns);
                if columns.len() == count {
                    columns.push(components::TableColumn { span, width });
                }
            } else {
                columns.push(components::TableColumn { span, width });
            }
        }
    }

    let mut columns = vec![];
    collect(TreeLinked::children(table), None, &mut columns);
    components::TableColumns { columns }
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
    fn parent_node(&self) -> Option<Self>;
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

    fn parent_node(&self) -> Option<Self> {
        crate::tree::TreeLinked::parent(&self)
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
    /// The fonts that the document can use, with its `@font-face` rules.
    fonts: Arc<super::text::font::FontContext>,
    font_resolver: FontResolver,
    /// Whether the traversal is inside a `<math>` element.
    in_math: bool,
    pub world: World,
    trim_next_inline_whitespace: bool,
    /// Whether the text before the next text node ends inside of a word, for
    /// `text-transform: capitalize`.
    in_word: bool,
    /// The directory that the `src` of an `<img>` is relative to.
    base_dir: PathBuf,
    /// The images that were loaded, by `src`, so that an image that is used often is embedded
    /// once. `None` for an image that could not be loaded.
    images: HashMap<String, Option<Arc<crate::image::LoadedImage>>>,
    /// The image of the `<svg>` element that is built next.
    pending_svg: Option<crate::image::LoadedImage>,
}

impl std::fmt::Debug for EntityConstructor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("EntityConstructor").finish()
    }
}

/// The height of a line of text in `font` with `style` and the distance of its baseline from the
/// top: the line height, with the half-leading above and below the font's own height.
fn line_metrics(font: &Font, style: &ComputedValues) -> (Px, Px) {
    let font_size = style.font.font_size;
    let font_height = font.scale_to_px(font.font_height(), font_size);
    let line_height = match style.font.line_height {
        LineHeight::Normal => font_height * 1.2,
        LineHeight::LengthPercentage(lp) => lp.absolute(font_size),
    };
    let baseline = font.scale_to_px(font.ascent(), font_size) + (line_height - font_height) / 2.0;
    (line_height, baseline)
}

impl EntityConstructor {
    pub fn new(mut world: World, base_dir: PathBuf) -> Self {
        world.register::<ComputedValues>();
        world.register::<InlineFormattingContext>();
        world.register::<BlockFormattingContext>();
        world.register::<Position>();
        world.register::<components::TextRun>();
        world.register::<components::RelativeOffset>();
        world.register::<components::ElementAlign>();
        world.register::<components::CanvasSize>();
        world.register::<components::VerticalAlignment>();
        world.register::<components::ListMarker>();
        world.register::<components::Decoration>();
        world.register::<MathNode>();
        world.register::<components::ImageBox>();
        world.register::<components::TableCellSpan>();
        world.register::<components::TableColumns>();
        world.insert(FontMap::new());
        world.insert(RootNode(None));
        world.insert(components::CanvasEntities::default());
        world.insert(components::TransformScopes::default());
        EntityConstructor {
            world,
            font_match_cache: Default::default(),
            fonts: super::text::font::FontContext::system(),
            font_resolver: Default::default(),
            in_math: false,
            trim_next_inline_whitespace: true,
            in_word: false,
            base_dir,
            images: Default::default(),
            pending_svg: None,
        }
    }

    /// Uses the fonts of the document, which are made from its `@font-face` rules.
    pub fn set_fonts(&mut self, fonts: Arc<super::text::font::FontContext>) {
        self.font_resolver.set_fonts(fonts.clone());
        self.font_match_cache.clear();
        self.fonts = fonts;
    }

    /// Records the transform of the node, if it has one, and that the node is inside of the
    /// transformed box that its parent is inside of.
    fn record_transform_scope(&mut self, node: &DomRef) {
        let Some(LayoutEntity(entity)) = node.layout_entity.get() else {
            return;
        };
        let Some(style) = NodeData::style(node) else {
            return;
        };
        let mut scopes = self.world.write_resource::<components::TransformScopes>();
        let parent_scope = node
            .parent()
            .and_then(|parent| parent.layout_entity.get())
            .and_then(|LayoutEntity(parent)| scopes.scope.get(&parent).copied());
        // a transform of an inline box that is not replaced does nothing
        let transformed = !style.box_.transform.0.is_empty()
            && !matches!(style.box_.display, Display::Inline);
        if transformed {
            scopes.boxes.insert(
                entity,
                components::TransformBox {
                    parent: parent_scope,
                    transform: style.box_.transform.clone(),
                    origin: style.box_.transform_origin,
                    font_size: (&*style.font).into(),
                },
            );
            scopes.scope.insert(entity, entity);
        } else if let Some(scope) = parent_scope {
            scopes.scope.insert(entity, scope);
        }
    }

    /// The boxes in `elements` that were not made from a node (anonymous blocks) are in what the
    /// box `container` is in.
    fn inherit_transform_scope(&self, container: Entity, elements: &[BlockLevelElement]) {
        let mut scopes = self.world.write_resource::<components::TransformScopes>();
        let Some(scope) = scopes.scope.get(&container).copied() else {
            return;
        };
        for element in elements {
            let entity = element.entity();
            scopes.scope.entry(entity).or_insert(scope);
        }
    }

    fn match_font(&mut self, style: &ComputedValues) -> Arc<Font> {
        let font_selector = FontSelector::from_style(&style);
        let fonts = &self.fonts;
        let mut font_map = self.world.write_resource::<FontMap>();
        let font_name = self
            .font_match_cache
            .entry(font_selector)
            .or_insert_with(|| {
                let font = fonts.match_font(&style);
                register_font(&mut font_map, font).postscript_name()
            });
        font_map[font_name].clone()
    }

    /// Like `match_font`, but prefers a font with an OpenType `MATH` table.
    fn match_math_font(&mut self, style: &ComputedValues) -> Arc<Font> {
        let font = self.fonts.match_math_font(style);
        let mut font_map = self.world.write_resource::<FontMap>();
        let name = register_font(&mut font_map, font).postscript_name();
        font_map[&name].clone()
    }

    /// The font and everything else that text with `style` is set in, with fallback fonts for the
    /// characters of `text`.
    fn text_face(&mut self, style: &ComputedValues, text: &str) -> (Arc<Font>, components::TextFace) {
        let font = self.match_font(style);
        let fallback = self.ensure_coverage(style, &font, text);
        let face = components::TextFace {
            font_name: font.postscript_name(),
            font_size: style.font.font_size,
            color: color_to_rgba(style.font.color),
            fallback,
        };
        (font, face)
    }

    /// Adds fallback fonts to `font` for the characters of `text` it has no glyph for.
    fn ensure_coverage(&mut self, style: &ComputedValues, font: &Font, text: &str) -> TextFonts {
        let mut font_map = self.world.write_resource::<FontMap>();
        let mut fonts = self
            .font_resolver
            .ensure_coverage(style, font, text, &mut font_map);
        // the spacing is in 1/1000 em, like the glyph advances
        let font_size = style.font.font_size.0;
        if font_size > 0.0 {
            fonts.letter_spacing = style.text.letter_spacing.0 / font_size * 1000.0;
        }
        fonts.features = crate::style::properties::font_features::Features::of(&style.font).into();
        if style.text.writing_mode != crate::style::properties::specified::WritingMode::HorizontalTb {
            fonts.orientation = Some(style.text.text_orientation);
        }
        fonts
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
                    self.in_word = false;
                }
                // the text in a box of its own does not continue a word of the text before it
                if style.box_.display != Display::Inline {
                    self.in_word = false;
                }

                let is_list_item =
                    element_data.name == QualName::new(None, ns!(html), local_name!("li"));
                let ordinal = if is_list_item {
                    list_item_ordinal(&node)
                } else {
                    0
                };
                let marker_text = style
                    .font
                    .list_style_type
                    .marker_text(ordinal)
                    .filter(|_| is_list_item);
                let marker = marker_text.map(|text| {
                    let (_, face) = self.text_face(&style, &text);
                    components::ListMarker { text, face }
                });

                if style.box_.display.is_atomic_inline() {
                    // its contents are a block of their own
                    self.trim_next_inline_whitespace = true;
                }
                let span = (style.box_.display == Display::TableCell)
                    .then(|| table_cell_span(element_data))
                    .flatten();
                let svg = self.pending_svg.take();
                // a `<canvas>` (and an `<iframe>`, whose document is not loaded) is a replaced box of 300 by 150 pixels, or the size of its
                // attributes, with nothing drawn in it (scripts do not run)
                let canvas = (element_data.name == QualName::new(None, ns!(html), local_name!("canvas"))
                    || element_data.name == QualName::new(None, ns!(html), local_name!("iframe")))
                .then(|| {
                    let dimension = |name, default: f32| {
                        element_data
                            .attributes
                            .borrow()
                            .get(&QualName::new(None, ns!(), name))
                            .and_then(|attribute| attribute.value.trim().parse::<f32>().ok())
                            .filter(|value| value.is_finite() && *value >= 0.0)
                            .unwrap_or(default)
                    };
                    (dimension(local_name!("width"), 300.0), dimension(local_name!("height"), 150.0))
                });
                let is_image = canvas.is_some()
                    || svg.is_some()
                    || element_data.name == QualName::new(None, ns!(html), local_name!("img"));
                let image = match svg {
                    Some(svg) => {
                        let (width, height) = svg.size();
                        Some(components::ImageBox {
                            image: Arc::new(svg),
                            width,
                            height,
                        })
                    }
                    None => is_image.then(|| self.image_of(element_data)).flatten(),
                };
                let mut style = style;
                if is_image && style.box_.display == Display::Inline {
                    // an inline replaced element is placed on a line as a whole
                    Arc::make_mut(&mut style.box_).display = Display::InlineBlock;
                }
                // an image is content: the white space after it is not at the start of the line
                if image.is_some() && style.box_.display == Display::InlineBlock {
                    self.trim_next_inline_whitespace = false;
                }
                let entity = self.build_default_entity(style);
                {
                    let mut canvas = self.world.write_resource::<components::CanvasEntities>();
                    if element_data.name == QualName::new(None, ns!(html), local_name!("html")) {
                        canvas.html.get_or_insert(entity.0);
                    } else if element_data.name == QualName::new(None, ns!(html), local_name!("body")) {
                        canvas.body.get_or_insert(entity.0);
                    }
                }
                if let Some(image) = image {
                    self.world
                        .write_storage::<components::ImageBox>()
                        .insert(entity.0, image)
                        .unwrap();
                }
                if let Some((width, height)) = canvas {
                    let ratioless = element_data.name == QualName::new(None, ns!(html), local_name!("iframe"));
                    self.world
                        .write_storage::<components::CanvasSize>()
                        .insert(entity.0, components::CanvasSize { width, height, ratioless })
                        .unwrap();
                }
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
                                Display::Inline
                                    | Display::InlineBlock
                                    | Display::InlineTable
                                    | Display::InlineFlex
                                    | Display::InlineGrid
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
                    let transformed = super::text::transform::apply(
                        &text,
                        style.text.text_transform,
                        &style.font.lang,
                        &mut self.in_word,
                    );
                    self.build_text_entity(style, transformed.as_deref().unwrap_or(&*text))
                }
            }
            // comments and the like have no box
            ParsedData::Comment(_) | ParsedData::ProcessingInstruction(..) => return,
            _ => self.build_default_entity(style),
        };

        node.assign_layout_entity(entity);
    }

    // called on top-down traversal
    fn build_root_entity(&mut self) -> LayoutEntity {
        self.build_default_entity(Default::default())
    }

    // called on top-down traversal
    /// The image of an `<img>` element, with its intrinsic size. An image that cannot be loaded is
    /// logged and leaves an empty box.
    fn image_of(&mut self, element_data: &ElementData) -> Option<components::ImageBox> {
        let attribute = |name| {
            element_data
                .attributes
                .borrow()
                .get(&QualName::new(None, ns!(), name))
                .map(|attribute| attribute.value.to_string())
        };
        let src = attribute(local_name!("src"))?;
        let image = self
            .images
            .entry(src.clone())
            .or_insert_with(|| match crate::image::load(&src, &self.base_dir, &self.fonts) {
                Ok(image) => Some(Arc::new(image)),
                Err(error) => {
                    log::warn!("cannot load the image `{}`: {}", src, error);
                    None
                }
            })
            .clone()?;
        // the `width` and `height` attributes are pixels (a percentage or a unit is not valid)
        let dimension = |name| {
            attribute(name)
                .and_then(|value| value.trim().parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0)
        };
        let (natural_width, natural_height) = image.size();
        let (width, height) = match (dimension(local_name!("width")), dimension(local_name!("height"))) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => (width, width * natural_height / natural_width),
            (None, Some(height)) => (height * natural_width / natural_height, height),
            (None, None) => (natural_width, natural_height),
        };
        Some(components::ImageBox {
            image,
            width,
            height,
        })
    }

    fn build_default_entity(&mut self, style: ComputedValues) -> LayoutEntity {
        self.world.create_entity().with(style).build().into()
    }

    /// The `vertical-align` of a box with its parent's font metrics (see
    /// [`components::AlignStep`]).
    fn align_step(
        &mut self,
        style: &ComputedValues,
        parent: &ComputedValues,
        atomic: bool,
    ) -> components::AlignStep {
        let font = self.match_font(style);
        let (line_height, baseline) = line_metrics(&font, style);
        let parent_font = self.match_font(parent);
        let parent_size = parent.font.font_size;
        components::AlignStep {
            align: style.box_.vertical_align,
            line_height,
            own: (!atomic).then_some((baseline, line_height - baseline)),
            parent_ascent: parent_font.scale_to_px(parent_font.ascent(), parent_size),
            parent_descent: Px(0.0) - parent_font.scale_to_px(parent_font.descent(), parent_size),
            parent_x_height: parent_font.x_height_px(parent_size),
            parent_font_size: parent_size,
        }
    }

    /// The ascent and descent of the strut of a block: a zero-width inline box with the block's
    /// font and line height. Line boxes start with it (CSS 2.1 §10.8.1), so a line without any
    /// inline content is still one line of the block's font tall.
    fn strut(&mut self, style: &ComputedValues) -> (Px, Px) {
        let font = self.match_font(style);
        let (line_height, baseline) = line_metrics(&font, style);
        (baseline, line_height - baseline)
    }

    fn build_text_entity(&mut self, style: ComputedValues, text: &str) -> LayoutEntity {
        let mut visible_text = String::with_capacity(text.len());
        let mut manual_hyphen_points = Vec::new();
        for character in text.chars() {
            if character == '\u{00ad}' {
                manual_hyphen_points.push(visible_text.len());
            } else {
                visible_text.push(character);
            }
        }
        let (font, face) = self.text_face(&style, &visible_text);
        // a hyphen is inserted when a word is hyphenated
        self.ensure_coverage(&style, &font, "-");

        let (line_height, baseline) = line_metrics(&font, &style);

        let decoration = style.font.text_decoration;
        let hyphens = style.text.hyphens;
        let lang = style.font.lang.clone();
        let entity = self
            .world
            .create_entity()
            .with(style)
            .with(components::TextRun {
                face,
                text: visible_text,
                line_height,
                baseline,
                decoration,
                hyphens,
                manual_hyphen_points,
                lang,
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

        // An image has no content: it stays a leaf box, which is as big as the image.
        if let LayoutEntity(entity) = node.layout_entity().unwrap()
            && (self.world.read_storage::<components::ImageBox>().contains(entity)
                || self.world.read_storage::<components::CanvasSize>().contains(entity))
        {
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
                InlineFormattingContext::new(elements.to_owned(), vec![], style, (Px(0.0), Px(0.0))),
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
        shifts: &mut InlineShifts,
        math_nodes: &specs::ReadStorage<MathNode>,
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
                if let Some(style) = child_style {
                    shifts.apply(elem, &[elem], style);
                }
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
                    if let Some(style) = child_style {
                        shifts.apply(elem, &elements, style);
                    }
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
                        shifts,
                        math_nodes,
                    );
                } else {
                    // a formula is placed on the line as a whole, like an `inline-block`
                    if let Some(style) = child_style.filter(|_| math_nodes.contains(elem)) {
                        shifts.apply(elem, &[elem], style);
                    }
                    inline_level_elements.push(elem);
                }
            }
        }
    }

    // called on bottom-up traversal
    fn build_block_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        // An `<input>` has no children: the text it shows comes from its attributes. It has to be
        // built before the storages below borrow the world.
        let replaced_text = replaced_element_text(node)
            .filter(|_| node.children().next().is_none())
            .map(|text| self.build_text_entity(node.style(), &text).0);
        // The strut is computed from the block's font, which needs the font map: like the text
        // above, it has to be built before the storages below borrow the world.
        let strut = self.strut(&node.style());

        let entities = self.world.entities();
        let mut block_formatting_contexts = self.world.write_storage::<BlockFormattingContext>();
        let mut inline_formatting_contexts = self.world.write_storage::<InlineFormattingContext>();
        let mut computed_values = self.world.write_storage::<ComputedValues>();
        let mut shifts = InlineShifts::fetch(&self.world);
        let math_nodes = self.world.read_storage::<MathNode>();

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
            &mut shifts,
            &math_nodes,
        );

        let has_block_level_elements = block_level_elements
            .iter()
            .any(|elem| !matches!(elem, BlockLevelElement::Absolute(_)));
        // A flex or grid container without content in the flow (only out-of-flow children, or
        // none) is still a container: its tracks give it a size and its out-of-flow children are
        // placed in them.
        let is_empty_container = style.box_.display.is_flex_or_grid()
            && inline_level_elements.is_empty()
            && replaced_text.is_none();
        if !has_block_level_elements && !is_empty_container {
            // no block-level elements were found, so we create a inline_formatting_context
            if inline_level_elements.is_empty()
                && let Some(text) = replaced_text
            {
                inline_level_elements.push(text);
            }
            let absolutes = block_level_elements
                .iter()
                .map(|elem| elem.entity())
                .collect();
            inline_formatting_contexts
                .insert(
                    entity,
                    InlineFormattingContext::new(inline_level_elements, absolutes, &style, strut),
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
        self.inherit_transform_scope(entity, &block_level_elements);

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

    fn shift_relatively(&self, owner: Entity, entities: &[Entity], style: &ComputedValues) {
        InlineShifts::fetch(&self.world).apply(owner, entities, style);
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let strut = self.strut(&node.style());
        let mut inline_formatting_context =
            InlineFormattingContext::new(vec![], vec![], &node.style(), strut);
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
                self.shift_relatively(child_entity, &[child_entity], &child_style);
                continue;
            }

            // merge child's InlineFormattingContext into the current context
            if let Some(InlineFormattingContext {
                elements,
                absolutes,
                ..
            }) = self.world.write_storage().remove(child_entity)
            {
                // a relatively positioned inline box moves everything in it
                self.shift_relatively(child_entity, &elements, &child_style);
                inline_formatting_context
                    .elements
                    .extend_from_slice(&elements);
                inline_formatting_context.absolutes.extend(absolutes);
                continue;
            }

            // a formula is placed on the line as a whole, like an `inline-block`
            if self.world.read_storage::<MathNode>().contains(child_entity) {
                self.shift_relatively(child_entity, &[child_entity], &child_style);
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
        use formel::host::{MathElement, NodeKind};
        use formel::unicode_math::StringExtMathml;

        let ParsedData::Element(element_data) = node.html_data() else {
            return;
        };
        let LayoutEntity(entity) = node.layout_entity().unwrap();
        let style = node.style();
        let font_name = self.match_math_font(&style).postscript_name();

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

/// Records the sizes of the `foreignObject` elements of an SVG tree in `islands`.
fn foreign_sizes(group: &usvg::Group, islands: &mut [IslandSpec]) {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => foreign_sizes(group, islands),
            usvg::Node::Foreign(foreign) => {
                if let Some(island) = islands.get_mut(foreign.handle() as usize) {
                    island.width = foreign.rect().width();
                    island.height = foreign.rect().height();
                }
            }
            usvg::Node::Image(image) => {
                if let usvg::ImageKind::SVG(tree) = image.kind() {
                    foreign_sizes(tree.root(), islands);
                }
            }
            _ => {}
        }
    }
}

/// What an inline box hands to the boxes and the text in it: its offsets (`position: relative`)
/// and its `vertical-align`. An inline box is not a box of the layout, so the line boxes move its
/// content.
struct InlineShifts<'a> {
    offsets: WriteStorage<'a, components::RelativeOffset>,
    aligns: WriteStorage<'a, components::VerticalAlignment>,
    steps: specs::ReadStorage<'a, components::ElementAlign>,
}

impl<'a> InlineShifts<'a> {
    fn fetch(world: &'a World) -> Self {
        InlineShifts {
            offsets: world.write_storage(),
            aligns: world.write_storage(),
            steps: world.read_storage(),
        }
    }

    /// Gives `entities`, the content of the inline box `owner` with the style `style`, the
    /// offsets and the alignment of the box.
    fn apply(&mut self, owner: Entity, entities: &[Entity], style: &ComputedValues) {
        if style.box_.position == PositionSpec::Relative {
            let pos = &style.position;
            let inset = components::Inset {
                left: pos.left,
                right: pos.right,
                top: pos.top,
                bottom: pos.bottom,
            };
            for &entity in entities {
                if let Ok(entry) = self.offsets.entry(entity) {
                    entry.or_insert_with(Default::default).0.push(inset);
                }
            }
        }
        if let Some(&components::ElementAlign(step)) = self.steps.get(owner) {
            for &entity in entities {
                if let Ok(entry) = self.aligns.entry(entity) {
                    entry.or_insert_with(Default::default).0.push(step);
                }
            }
        }
    }
}

/// The presentational hint of the `width` and `height` attributes of an `<img>` or `<iframe>`: a number of
/// pixels or a percentage (HTML 2.4.4.4 "rules for parsing dimension values": what follows the
/// number, apart from a `%`, is ignored).
fn image_size_hint(element: &ElementData) -> Option<PropertyDeclarationBlock> {
    if element.name.local != local_name!("img") && element.name.local != local_name!("iframe") {
        return None;
    }
    let attributes = element.attributes.borrow();
    let mut css = String::new();
    let mut has_percentage = false;
    for name in [local_name!("width"), local_name!("height")] {
        let Some(attribute) = attributes.get(&QualName::new(None, ns!(), name.clone())) else {
            continue;
        };
        let value = attribute.value.trim_start();
        let end = value
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(value.len());
        let Ok(number) = value[..end].parse::<f32>() else {
            continue;
        };
        if !number.is_finite() {
            continue;
        }
        let unit = if value[end..].starts_with('%') { "%" } else { "px" };
        has_percentage |= unit == "%";
        css.push_str(&format!("{}: {}{};", &*name, number, unit));
    }
    // pixels are the size of the image box itself (see `image_of`)
    has_percentage.then(|| PropertyDeclarationBlock::parse_inline(&css))
}

/// The presentational hints of the `cellspacing` attribute of a `<table>` (`border-spacing`) and
/// of its `cellpadding` attribute, which is the `padding` of the cells of the table.
fn table_hint(node: &DomRef, element: &ElementData) -> Option<PropertyDeclarationBlock> {
    let number = |element: &ElementData, name| -> Option<u32> {
        let attributes = element.attributes.borrow();
        let value = attributes.get(&QualName::new(None, ns!(), name))?.value.to_string();
        let value = value.trim_start();
        let end = value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len());
        value[..end].parse().ok()
    };
    let local = &element.name.local;
    if *local == local_name!("table") {
        let spacing = number(element, local_name!("cellspacing"))?;
        return Some(PropertyDeclarationBlock::parse_inline(&format!("border-spacing: {}px", spacing)));
    }
    if *local == local_name!("td") || *local == local_name!("th") {
        let mut ancestor = node.parent();
        while let Some(parent) = ancestor {
            if let Some(parent_element) = parent.clone().into_element_ref() {
                if parent_element.name.local == local_name!("table") {
                    let padding = number(&parent_element, local_name!("cellpadding"))?;
                    return Some(PropertyDeclarationBlock::parse_inline(&format!("padding: {}px", padding)));
                }
            }
            ancestor = parent.parent();
        }
    }
    None
}
