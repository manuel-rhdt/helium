/// ECS read/write bindings for the Scheme interpreter.
///
/// Read-only components are captured in Rc<ReadStorage> closures.
/// Writable components use Rc<RefCell<WriteStorage>>.
/// All writes happen inline during eval; no deferred write log.

use std::{cell::RefCell, rc::Rc};

use specs::{
    prelude::{Entities, Join, Read, ReadStorage, SystemData, Write, WriteStorage},
    Entity, World, WorldExt,
};

use crate::{
    layout::{
        components::{
            AbsoluteGroup, AbsoluteGroups, BlockLevelElement, Decoration, EdgeValues, IntrinsicSize,
            Position, TextList, TextRun, ZIndex,
        },
        absolute,
        inline::InlineFormattingContext,
        normal_formatting::{BlockFormattingContext, compute_element_dimensions},
    },
    style::properties::computed::{
        ComputedValues, GenericSize, LengthPercentage, PixelLength, RGBA,
    },
};

use super::{EnvRef, RuntimeError, Value};

// ---------------------------------------------------------------------------
// Entity ID encoding
// ---------------------------------------------------------------------------

pub fn entity_to_value<'w>(ent: Entity) -> Value<'w> {
    Value::from(ent)
}

/// An entity reference as received from Scheme: the entity id together with its generation.
/// The generation makes sure that a stale reference to a deleted entity whose id has been
/// reused is not mistaken for the new entity.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct EntityRef {
    id: u32,
    generation: i32,
}

impl std::fmt::Display for EntityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(entity {} generation {})", self.id, self.generation)
    }
}

fn value_to_entity_ref(val: &Value) -> Result<EntityRef, RuntimeError> {
    match val {
        Value::List(items) if items.len() == 2 => Ok(EntityRef {
            id: num(&items[0])? as u32,
            generation: num(&items[1])? as i32,
        }),
        _ => Err(RuntimeError::Message(format!(
            "entity: expected (id generation) list, got {:?}", val
        ))),
    }
}

/// Finds the alive entity that `entity_ref` refers to. Returns `None` if the entity was deleted
/// or the id has been reused by a different entity since the reference was created.
fn find_entity(entities: &Entities, entity_ref: EntityRef) -> Option<Entity> {
    let entity = entities.entity(entity_ref.id);
    (entity.r#gen().id() == entity_ref.generation && entities.is_alive(entity)).then_some(entity)
}

// ---------------------------------------------------------------------------
// Component → Value conversions
// ---------------------------------------------------------------------------

fn px<'w>(v: PixelLength) -> Value<'w> {
    Value::Number(v.0 as f64)
}

fn position_to_value<'w>(pos: &Position) -> Value<'w> {
    Value::List(vec![
        px(pos.x), px(pos.y), px(pos.width), px(pos.height),
        Value::Number(pos.z.0 as f64),
        Value::Number(pos.page as f64),
    ])
}

fn text_run_to_value<'w>(tr: &TextRun) -> Value<'w> {
    Value::List(vec![
        Value::String(tr.text.clone()),
        Value::String(tr.font_name.clone()),
        px(tr.font_size), px(tr.line_height), px(tr.baseline),
    ])
}

fn edge_values_to_value<'w>(e: EdgeValues<PixelLength>) -> Value<'w> {
    Value::List(vec![px(e.top), px(e.right), px(e.bottom), px(e.left)])
}

fn rgba_to_value<'w>(c: RGBA) -> Value<'w> {
    Value::List(vec![
        Value::Number(c.red as f64), Value::Number(c.green as f64),
        Value::Number(c.blue as f64), Value::Number(c.alpha as f64),
    ])
}

pub fn decoration_to_value<'w>(d: &Decoration) -> Value<'w> {
    Value::List(vec![
        edge_values_to_value(d.margin),
        edge_values_to_value(d.padding),
        edge_values_to_value(d.border),
        rgba_to_value(d.background_color),
        rgba_to_value(d.border_color.top),
    ])
}

fn value_to_decoration(v: &Value) -> Result<Decoration, RuntimeError> {
    match v {
        Value::List(items) if items.len() == 5 => {
            let margin  = value_to_edges(&items[0])?;
            let padding = value_to_edges(&items[1])?;
            let border  = value_to_edges(&items[2])?;
            let bg      = value_to_rgba(&items[3])?;
            let bc      = value_to_rgba(&items[4])?;
            Ok(Decoration { margin, padding, border, background_color: bg, border_color: EdgeValues::all(bc) })
        }
        _ => Err(RuntimeError::Message(format!(
            "expected decoration (5-element list), got {:?}", v
        ))),
    }
}

fn value_to_edges(v: &Value) -> Result<EdgeValues<PixelLength>, RuntimeError> {
    match v {
        Value::List(items) if items.len() == 4 => Ok(EdgeValues {
            top:    PixelLength(num(&items[0])? as f32),
            right:  PixelLength(num(&items[1])? as f32),
            bottom: PixelLength(num(&items[2])? as f32),
            left:   PixelLength(num(&items[3])? as f32),
        }),
        _ => Err(RuntimeError::Message(format!(
            "expected edge values (4-element list), got {:?}", v
        ))),
    }
}

fn value_to_rgba(v: &Value) -> Result<RGBA, RuntimeError> {
    match v {
        Value::List(items) if items.len() == 4 => Ok(RGBA {
            red:   num(&items[0])? as u8,
            green: num(&items[1])? as u8,
            blue:  num(&items[2])? as u8,
            alpha: num(&items[3])? as u8,
        }),
        _ => Err(RuntimeError::Message(format!(
            "expected RGBA (4-element list), got {:?}", v
        ))),
    }
}

fn intrinsic_size_to_value<'w>(is: &IntrinsicSize) -> Value<'w> {
    match *is {
        IntrinsicSize::Width(w) => Value::List(vec![px(w), Value::Bool(false), Value::Bool(false)]),
        IntrinsicSize::Height(h) => Value::List(vec![Value::Bool(false), px(h), Value::Bool(false)]),
        IntrinsicSize::Size { width, height } => Value::List(vec![px(width), px(height), Value::Bool(false)]),
        IntrinsicSize::SizeWithBaseline { width, height, baseline } => {
            Value::List(vec![px(width), px(height), px(baseline)])
        }
    }
}

fn inline_context_to_value<'w>(ictx: &InlineFormattingContext) -> Value<'w> {
    let elements: Vec<Value<'w>> = ictx.elements.iter().map(|&e| entity_to_value(e)).collect();
    Value::List(elements)
}

fn block_level_element_to_value<'w>(elem: BlockLevelElement) -> Value<'w> {
    let (tag, ent) = match elem {
        BlockLevelElement::ContainerStart(e) => ("container-start", e),
        BlockLevelElement::Absolute(e)       => ("absolute", e),
        BlockLevelElement::Leaf(e)           => ("leaf", e),
        BlockLevelElement::ContainerEnd(e)   => ("container-end", e),
    };
    Value::List(vec![Value::Symbol(tag.to_string()), entity_to_value(ent)])
}

fn text_align_to_value<'w>(align: crate::style::properties::specified::TextAlign) -> Value<'w> {
    use crate::style::properties::specified::TextAlign;
    let s = match align {
        TextAlign::Left    => "left",
        TextAlign::Right   => "right",
        TextAlign::Center  => "center",
        TextAlign::Justify => "justify",
    };
    Value::Symbol(s.to_string())
}

// ---------------------------------------------------------------------------
// setup_ecs_env
// ---------------------------------------------------------------------------

pub fn setup_ecs_env<'world>(env: &EnvRef<'world>, world: &'world mut World) {
    world.register::<Position>();
    world.register::<TextRun>();
    world.register::<Decoration>();
    world.register::<IntrinsicSize>();
    world.register::<InlineFormattingContext>();
    world.register::<BlockFormattingContext>();
    if !world.has_value::<AbsoluteGroups>() {
        world.insert(AbsoluteGroups::default());
    }

    // Read-only storages
    let text_runs       = Rc::new(ReadStorage::<'world, TextRun>::fetch(world));
    let computed_values = Rc::new(ReadStorage::<'world, ComputedValues>::fetch(world));
    let intrinsic_sizes = Rc::new(ReadStorage::<'world, IntrinsicSize>::fetch(world));
    let block_ctxs      = Rc::new(ReadStorage::<'world, BlockFormattingContext>::fetch(world));
    let entities        = Rc::new(Entities::<'world>::fetch(world));
    let root_node       = Rc::new(Read::<'world, crate::layout::components::RootNode>::fetch(world));

    // Read-write storages
    let positions    = Rc::new(RefCell::new(WriteStorage::<'world, Position>::fetch(world)));
    let decorations  = Rc::new(RefCell::new(WriteStorage::<'world, Decoration>::fetch(world)));
    let inline_ctxs  = Rc::new(RefCell::new(WriteStorage::<'world, InlineFormattingContext>::fetch(world)));
    let text_list    = Rc::new(RefCell::new(Write::<'world, TextList>::fetch(world)));
    let abs_groups   = Rc::new(RefCell::new(Write::<'world, AbsoluteGroups>::fetch(world)));
    // indices of the absolute groups that are currently being laid out, innermost last
    let group_stack: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));

    // --- Query bindings ---

    {
        let entities = Rc::clone(&entities);
        inject(env, "all-entities", move |_args| {
            let list: Vec<Value<'world>> = (&*entities).join().map(entity_to_value).collect();
            Ok(Value::List(list))
        });
    }

    {
        let root_node = Rc::clone(&root_node);
        inject(env, "root-entity", move |_args| match root_node.0 {
            Some(ent) => Ok(entity_to_value(ent)),
            None => Ok(Value::Bool(false)),
        });
    }

    // (entity-position entity-id) → (x y w h z page) | #f
    {
        let positions = Rc::clone(&positions);
        let entities  = Rc::clone(&entities);
        inject(env, "entity-position", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            let positions_ref = positions.borrow();
            match ent.and_then(|e| positions_ref.get(e)) {
                Some(pos) => Ok(position_to_value(pos)),
                None      => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-text-run entity-id) → list | #f
    {
        let text_runs = Rc::clone(&text_runs);
        let entities  = Rc::clone(&entities);
        inject(env, "entity-text-run", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| text_runs.get(e)) {
                Some(tr) => Ok(text_run_to_value(tr)),
                None     => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-decoration entity-id) → list | #f
    {
        let decorations = Rc::clone(&decorations);
        let entities    = Rc::clone(&entities);
        inject(env, "entity-decoration", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            let decorations_ref = decorations.borrow();
            match ent.and_then(|e| decorations_ref.get(e)) {
                Some(d) => Ok(decoration_to_value(d)),
                None    => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-intrinsic-size entity-id) → (w h baseline) | #f
    {
        let intrinsic_sizes = Rc::clone(&intrinsic_sizes);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-intrinsic-size", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| intrinsic_sizes.get(e)) {
                Some(is) => Ok(intrinsic_size_to_value(is)),
                None     => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-inline-context entity-id) → (list entity-id ...) | #f
    {
        let inline_ctxs = Rc::clone(&inline_ctxs);
        let entities    = Rc::clone(&entities);
        inject(env, "entity-inline-context", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            let inline_ref = inline_ctxs.borrow();
            match ent.and_then(|e| inline_ref.get(e)) {
                Some(ictx) => Ok(inline_context_to_value(ictx)),
                None       => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-has-inline-context entity-id) → #t | #f
    {
        let inline_ctxs = Rc::clone(&inline_ctxs);
        let entities    = Rc::clone(&entities);
        inject(env, "entity-has-inline-context", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            let ent = ent.ok_or_else(|| RuntimeError::Message(
                format!("entity-has-inline-context: entity {} not found", id)
            ))?;
            Ok(Value::Bool(inline_ctxs.borrow().contains(ent)))
        });
    }

    // (entity-block-context entity-id) → list of (type entity-id) | #f
    {
        let block_ctxs = Rc::clone(&block_ctxs);
        let entities   = Rc::clone(&entities);
        inject(env, "entity-block-context", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| block_ctxs.get(e)) {
                Some(ctx) => {
                    let list: Vec<Value<'world>> = ctx.elements.iter()
                        .map(|&elem| block_level_element_to_value(elem))
                        .collect();
                    Ok(Value::List(list))
                }
                None => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-compute-dims entity-id container-width) → (decoration inline-size inline-offset) | #f
    {
        let computed_values = Rc::clone(&computed_values);
        let intrinsic_sizes = Rc::clone(&intrinsic_sizes);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-compute-dims", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message(
                    "entity-compute-dims requires 2 arguments".to_string()
                ));
            }
            let id = value_to_entity_ref(&args[0])?;
            let container_width = PixelLength(num(&args[1])? as f32);
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| computed_values.get(e).map(|cv| (e, cv))) {
                Some((e, style)) => {
                    let intrinsic = intrinsic_sizes.get(e).cloned();
                    let dims = compute_element_dimensions(container_width, intrinsic, style);
                    Ok(Value::List(vec![
                        decoration_to_value(&dims.decoration),
                        px(dims.inline_size),
                        px(dims.inline_offset()),
                    ]))
                }
                None => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-css-height entity-id parent-width) → number | #f
    {
        let computed_values = Rc::clone(&computed_values);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-css-height", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message(
                    "entity-css-height requires 2 arguments".to_string()
                ));
            }
            let id = value_to_entity_ref(&args[0])?;
            let parent_width = PixelLength(num(&args[1])? as f32);
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| computed_values.get(e)) {
                Some(style) => match style.position.height {
                    GenericSize::Auto => Ok(Value::Bool(false)),
                    GenericSize::LengthPercentage(lp) => Ok(px(lp.absolute(parent_width))),
                },
                None => Ok(Value::Bool(false)),
            }
        });
    }

    // --- Inline layout bindings ---

    // (entity-inline-text-align entity-id) → symbol | #f
    {
        let inline_ctxs = Rc::clone(&inline_ctxs);
        let entities    = Rc::clone(&entities);
        inject(env, "entity-inline-text-align", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id);
            let inline_ref = inline_ctxs.borrow();
            match ent.and_then(|e| inline_ref.get(e)) {
                Some(ifm) => Ok(text_align_to_value(ifm.text_align)),
                None      => Ok(Value::Bool(false)),
            }
        });
    }

    // (entity-text-indent entity-id line-width) → number
    {
        let computed_values = Rc::clone(&computed_values);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-text-indent", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message("entity-text-indent requires 2 args".to_string()));
            }
            let id = value_to_entity_ref(&args[0])?;
            let line_width = PixelLength(num(&args[1])? as f32);
            let ent = find_entity(&entities, id);
            match ent.and_then(|e| computed_values.get(e)) {
                Some(style) => Ok(px(style.text.text_indent.absolute(line_width))),
                None        => Ok(Value::Number(0.0)),
            }
        });
    }

    // (layout-inline-lines entity-id line-width indent) → list of lines
    // Breaks the paragraph of an inline context into lines (Knuth-Plass, in Rust), creates one
    // entity per line and stores the lines in the inline context.
    // Each line is (from-break to-break height baseline line-entity-id content-width).
    {
        let inline_ctxs     = Rc::clone(&inline_ctxs);
        let text_runs       = Rc::clone(&text_runs);
        let intrinsic_sizes = Rc::clone(&intrinsic_sizes);
        let entities        = Rc::clone(&entities);
        inject(env, "layout-inline-lines", move |args| {
            if args.len() != 3 {
                return Err(RuntimeError::Message(
                    "layout-inline-lines requires 3 args: entity line-width indent".to_string()
                ));
            }
            let id         = value_to_entity_ref(&args[0])?;
            let line_width = PixelLength(num(&args[1])? as f32);
            let indent     = PixelLength(num(&args[2])? as f32);
            let ent = find_entity(&entities, id)
                .ok_or_else(|| RuntimeError::Message(
                    format!("layout-inline-lines: entity {} not found", id)
                ))?;
            let mut inline_mut = inline_ctxs.borrow_mut();
            let ifm = inline_mut.get_mut(ent).ok_or_else(|| RuntimeError::Message(
                format!("layout-inline-lines: entity {} has no inline context", id)
            ))?;
            ifm.break_lines(line_width, indent, &*entities, &*text_runs, &*intrinsic_sizes)
                .map_err(|e| RuntimeError::Message(format!("layout-inline-lines: {}", e)))?;
            let lines = ifm.lines.iter().map(|line| {
                Value::List(vec![
                    Value::Number(line.from_break as f64),
                    Value::Number(line.to_break as f64),
                    px(line.height),
                    px(line.baseline),
                    entity_to_value(line.entity),
                    px(ifm.content_width(line.from_break, line.to_break)),
                ])
            }).collect();
            Ok(Value::List(lines))
        });
    }

    // --- Absolutely positioned boxes ---

    // (entity-inner-context entity-id) → list of (type entity-id)
    // The flow inside of a box that is laid out on its own: the contents of its block formatting
    // context without the box itself, or its lines if it only contains inline content.
    {
        let block_ctxs  = Rc::clone(&block_ctxs);
        let inline_ctxs = Rc::clone(&inline_ctxs);
        let entities    = Rc::clone(&entities);
        inject(env, "entity-inner-context", move |args| {
            let id = value_to_entity_ref(args.get(0).unwrap_or(&Value::Null))?;
            let ent = find_entity(&entities, id).ok_or_else(|| RuntimeError::Message(
                format!("entity-inner-context: entity {} not found", id)
            ))?;
            Ok(Value::List(inner_context(ent, &block_ctxs, &inline_ctxs)
                .into_iter()
                .map(block_level_element_to_value)
                .collect()))
        });
    }

    // (entity-absolute-decoration entity-id containing-block-width) → decoration
    // Padding, border and colors of an absolutely positioned box. The margins are 0.
    {
        let computed_values = Rc::clone(&computed_values);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-absolute-decoration", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message("entity-absolute-decoration requires 2 arguments".to_string()));
            }
            let id = value_to_entity_ref(&args[0])?;
            let cb_width = PixelLength(num(&args[1])? as f32);
            let style = find_entity(&entities, id).and_then(|e| computed_values.get(e)).ok_or_else(|| {
                RuntimeError::Message(format!("entity-absolute-decoration: entity {} not found", id))
            })?;
            let mut deco = crate::layout::inline_size::calculate_padding_and_border(cb_width, style);
            deco.margin = EdgeValues::all(PixelLength(0.0));
            Ok(decoration_to_value(&deco))
        });
    }

    // (entity-absolute-horizontal entity-id cb-width static-left)
    //     → (left width margin-left margin-right)
    {
        let computed_values = Rc::clone(&computed_values);
        let block_ctxs      = Rc::clone(&block_ctxs);
        let inline_ctxs     = Rc::clone(&inline_ctxs);
        let intrinsic_sizes = Rc::clone(&intrinsic_sizes);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-absolute-horizontal", move |args| {
            if args.len() != 3 {
                return Err(RuntimeError::Message("entity-absolute-horizontal requires 3 arguments".to_string()));
            }
            let id = value_to_entity_ref(&args[0])?;
            let cb_width = num(&args[1])? as f32;
            let static_left = num(&args[2])? as f32;
            let ent = find_entity(&entities, id).ok_or_else(|| RuntimeError::Message(
                format!("entity-absolute-horizontal: entity {} not found", id)
            ))?;
            let style = computed_values.get(ent).ok_or_else(|| RuntimeError::Message(
                format!("entity-absolute-horizontal: entity {} has no style", id)
            ))?;
            let pos = &style.position;
            let basis = PixelLength(cb_width);
            let resolve = |size: GenericSize| match size {
                GenericSize::Auto => None,
                GenericSize::LengthPercentage(lp) => Some(lp.absolute(basis).0),
            };
            let extras = pos.padding_left.absolute(basis).0
                + pos.padding_right.absolute(basis).0
                + pos.border_left_width.0
                + pos.border_right_width.0;
            let axis = absolute::Axis {
                container: cb_width,
                start: resolve(pos.left),
                end: resolve(pos.right),
                size: resolve(pos.width),
                margin_start: resolve(pos.margin_left),
                margin_end: resolve(pos.margin_right),
                extras,
                static_start: static_left,
            };
            let intrinsic = IntrinsicWidths {
                block_ctxs: &block_ctxs,
                inline_ctxs: &inline_ctxs,
                computed_values: &computed_values,
                intrinsic_sizes: &intrinsic_sizes,
            };
            let result = absolute::solve_horizontal(axis, |available| {
                let (min, max) = intrinsic.of_contents(ent);
                absolute::shrink_to_fit(min, max, available)
            });
            Ok(Value::List(vec![
                Value::Number(result.start as f64),
                Value::Number(result.size as f64),
                Value::Number(result.margin_start as f64),
                Value::Number(result.margin_end as f64),
            ]))
        });
    }

    // (entity-absolute-vertical entity-id cb-width cb-height static-top content-height)
    //     → (top height margin-top margin-bottom top-is-auto)
    {
        let computed_values = Rc::clone(&computed_values);
        let entities        = Rc::clone(&entities);
        inject(env, "entity-absolute-vertical", move |args| {
            if args.len() != 5 {
                return Err(RuntimeError::Message("entity-absolute-vertical requires 5 arguments".to_string()));
            }
            let id = value_to_entity_ref(&args[0])?;
            let cb_width = PixelLength(num(&args[1])? as f32);
            let cb_height = num(&args[2])? as f32;
            let static_top = num(&args[3])? as f32;
            let content_height = num(&args[4])? as f32;
            let style = find_entity(&entities, id).and_then(|e| computed_values.get(e)).ok_or_else(|| {
                RuntimeError::Message(format!("entity-absolute-vertical: entity {} not found", id))
            })?;
            let pos = &style.position;
            let basis = PixelLength(cb_height);
            let resolve = |size: GenericSize| match size {
                GenericSize::Auto => None,
                GenericSize::LengthPercentage(lp) => Some(lp.absolute(basis).0),
            };
            // percentages in margins refer to the width of the containing block
            let resolve_margin = |size: GenericSize| match size {
                GenericSize::Auto => None,
                GenericSize::LengthPercentage(lp) => Some(lp.absolute(cb_width).0),
            };
            let extras = pos.padding_top.absolute(cb_width).0
                + pos.padding_bottom.absolute(cb_width).0
                + pos.border_top_width.0
                + pos.border_bottom_width.0;
            let axis = absolute::Axis {
                container: cb_height,
                start: resolve(pos.top),
                end: resolve(pos.bottom),
                size: resolve(pos.height),
                margin_start: resolve_margin(pos.margin_top),
                margin_end: resolve_margin(pos.margin_bottom),
                extras,
                static_start: static_top,
            };
            let result = absolute::solve_vertical(axis, content_height);
            Ok(Value::List(vec![
                Value::Number(result.start as f64),
                Value::Number(result.size as f64),
                Value::Number(result.margin_start as f64),
                Value::Number(result.margin_end as f64),
                Value::Bool(axis.start.is_none()),
            ]))
        });
    }

    // (begin-absolute-group! containing-block-entity-or-#f)
    // From now on every entity that gets a position belongs to a new group of an absolutely
    // positioned box. Groups can be nested.
    {
        let abs_groups  = Rc::clone(&abs_groups);
        let group_stack = Rc::clone(&group_stack);
        let entities    = Rc::clone(&entities);
        inject(env, "begin-absolute-group!", move |args| {
            let containing_block = match args.get(0) {
                Some(Value::Bool(false)) | None => None,
                Some(value) => {
                    let id = value_to_entity_ref(value)?;
                    Some(find_entity(&entities, id).ok_or_else(|| RuntimeError::Message(
                        format!("begin-absolute-group!: entity {} not found", id)
                    ))?)
                }
            };
            let mut groups = abs_groups.borrow_mut();
            groups.groups.push(AbsoluteGroup { containing_block, entities: vec![] });
            group_stack.borrow_mut().push(groups.groups.len() - 1);
            Ok(Value::Null)
        });
    }

    // (set-group-containing-block! entity-or-#f)
    // Changes the entity whose page the innermost open group is placed on.
    {
        let abs_groups  = Rc::clone(&abs_groups);
        let group_stack = Rc::clone(&group_stack);
        let entities    = Rc::clone(&entities);
        inject(env, "set-group-containing-block!", move |args| {
            let containing_block = match args.get(0) {
                Some(Value::Bool(false)) | None => None,
                Some(value) => {
                    let id = value_to_entity_ref(value)?;
                    Some(find_entity(&entities, id).ok_or_else(|| RuntimeError::Message(
                        format!("set-group-containing-block!: entity {} not found", id)
                    ))?)
                }
            };
            let group = *group_stack.borrow().last().ok_or_else(|| RuntimeError::Message(
                "set-group-containing-block!: no group is open".to_string()
            ))?;
            abs_groups.borrow_mut().groups[group].containing_block = containing_block;
            Ok(Value::Null)
        });
    }

    // (end-absolute-group!)
    {
        let group_stack = Rc::clone(&group_stack);
        inject(env, "end-absolute-group!", move |_args| {
            group_stack.borrow_mut().pop().ok_or_else(|| RuntimeError::Message(
                "end-absolute-group!: no group is open".to_string()
            ))?;
            Ok(Value::Null)
        });
    }

    // (translate-group! dy)
    // Moves everything in the innermost open group down by dy.
    {
        let abs_groups  = Rc::clone(&abs_groups);
        let group_stack = Rc::clone(&group_stack);
        let positions   = Rc::clone(&positions);
        inject(env, "translate-group!", move |args| {
            let dy = PixelLength(num(args.get(0).unwrap_or(&Value::Null))? as f32);
            let group = *group_stack.borrow().last().ok_or_else(|| RuntimeError::Message(
                "translate-group!: no group is open".to_string()
            ))?;
            let groups = abs_groups.borrow();
            let mut positions = positions.borrow_mut();
            for &entity in &groups.groups[group].entities {
                if let Some(pos) = positions.get_mut(entity) {
                    pos.y += dy;
                }
            }
            Ok(Value::Null)
        });
    }

    // --- Write bindings ---

    // (set-position! entity-id x y width height z page)
    {
        let positions = Rc::clone(&positions);
        let entities  = Rc::clone(&entities);
        let abs_groups  = Rc::clone(&abs_groups);
        let group_stack = Rc::clone(&group_stack);
        inject(env, "set-position!", move |args| {
            if args.len() != 7 {
                return Err(RuntimeError::Message(
                    "set-position! requires 7 args: entity x y w h z page".to_string()
                ));
            }
            let entity_id = value_to_entity_ref(&args[0])?;
            let x      = num(&args[1])? as f32;
            let y      = num(&args[2])? as f32;
            let width  = num(&args[3])? as f32;
            let height = num(&args[4])? as f32;
            let z      = num(&args[5])? as i64;
            let page   = num(&args[6])? as usize;
            let ent = find_entity(&entities, entity_id)
                .ok_or_else(|| RuntimeError::Message(
                    format!("set-position!: entity {} not found", entity_id)
                ))?;
            if let Some(&group) = group_stack.borrow().last() {
                let mut groups = abs_groups.borrow_mut();
                let entities = &mut groups.groups[group].entities;
                if !entities.contains(&ent) {
                    entities.push(ent);
                }
            }
            let mut positions = positions.borrow_mut();
            let entry = positions.entry(ent).map_err(|e| RuntimeError::Message(
                format!("set-position!: {}", e)
            ))?;
            entry.replace(Position {
                x: PixelLength(x), y: PixelLength(y),
                width: PixelLength(width), height: PixelLength(height),
                z: ZIndex(z), page,
            });
            Ok(Value::Null)
        });
    }

    // (set-decoration! entity-id decoration)
    {
        let decorations = Rc::clone(&decorations);
        let entities    = Rc::clone(&entities);
        inject(env, "set-decoration!", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message(
                    "set-decoration! requires 2 args".to_string()
                ));
            }
            let entity_id  = value_to_entity_ref(&args[0])?;
            let decoration = value_to_decoration(&args[1])?;
            let ent = find_entity(&entities, entity_id)
                .ok_or_else(|| RuntimeError::Message(
                    format!("set-decoration!: entity {} not found", entity_id)
                ))?;
            decorations.borrow_mut().insert(ent, decoration).map_err(|e| RuntimeError::Message(
                format!("set-decoration!: {}", e)
            ))?;
            Ok(Value::Null)
        });
    }

    // (append-to-text-list! type entity-id)
    {
        let text_list = Rc::clone(&text_list);
        let entities  = Rc::clone(&entities);
        let group_stack = Rc::clone(&group_stack);
        inject(env, "append-to-text-list!", move |args| {
            if args.len() != 2 {
                return Err(RuntimeError::Message(
                    "append-to-text-list! requires 2 args".to_string()
                ));
            }
            let id  = value_to_entity_ref(&args[1])?;
            let ent = find_entity(&entities, id)
                .ok_or_else(|| RuntimeError::Message(
                    format!("append-to-text-list!: entity {} not found", id)
                ))?;
            let elem = match &args[0] {
                Value::Symbol(s) if s == "container-start" => BlockLevelElement::ContainerStart(ent),
                Value::Symbol(s) if s == "leaf"            => BlockLevelElement::Leaf(ent),
                Value::Symbol(s) if s == "container-end"   => BlockLevelElement::ContainerEnd(ent),
                other => return Err(RuntimeError::Message(
                    format!("append-to-text-list!: unknown type {:?}", other)
                )),
            };
            // absolutely positioned boxes are not paginated with the main flow
            if group_stack.borrow().is_empty() {
                text_list.borrow_mut().list.push(elem);
            }
            Ok(Value::Null)
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------


/// The flow of a box that is laid out on its own, see `entity-inner-context`.
fn inner_context(
    entity: Entity,
    block_ctxs: &ReadStorage<BlockFormattingContext>,
    inline_ctxs: &RefCell<WriteStorage<InlineFormattingContext>>,
) -> Vec<BlockLevelElement> {
    if let Some(ctx) = block_ctxs.get(entity) {
        // drop the container start and end of the box itself
        let elements = &ctx.elements;
        return elements[1.min(elements.len())..elements.len().saturating_sub(1)].to_vec();
    }
    let inline_ctxs = inline_ctxs.borrow();
    match inline_ctxs.get(entity) {
        Some(ifc) => ifc
            .absolutes
            .iter()
            .map(|&a| BlockLevelElement::Absolute(a))
            .chain(std::iter::once(BlockLevelElement::Leaf(entity)))
            .collect(),
        None => vec![],
    }
}

/// Computes the minimum and maximum content width of boxes (CSS Sizing: min-content and
/// max-content contributions), needed for shrink-to-fit widths.
struct IntrinsicWidths<'a, 'w> {
    block_ctxs: &'a ReadStorage<'w, BlockFormattingContext>,
    inline_ctxs: &'a RefCell<WriteStorage<'w, InlineFormattingContext>>,
    computed_values: &'a ReadStorage<'w, ComputedValues>,
    intrinsic_sizes: &'a ReadStorage<'w, IntrinsicSize>,
}

impl IntrinsicWidths<'_, '_> {
    /// `(min-content, max-content)` width of the content box of `entity`.
    fn of_contents(&self, entity: Entity) -> (f32, f32) {
        let (mut min, mut max) = (0.0f32, 0.0f32);
        // horizontal margin, border and padding of the open containers
        let mut open_extras: Vec<f32> = Vec::new();
        // > 0 while we are inside of a container with a fixed width whose contents don't matter
        let mut skipping = 0usize;

        for element in inner_context(entity, self.block_ctxs, self.inline_ctxs) {
            match element {
                BlockLevelElement::ContainerStart(e) if skipping > 0 => {
                    let _ = e;
                    skipping += 1;
                }
                BlockLevelElement::ContainerEnd(_) if skipping > 0 => skipping -= 1,
                _ if skipping > 0 => {}
                BlockLevelElement::ContainerStart(e) => {
                    let (extras, fixed_width) = self.box_extras(e);
                    if let Some(width) = fixed_width {
                        let total = open_extras.iter().sum::<f32>() + extras + width;
                        min = min.max(total);
                        max = max.max(total);
                        skipping = 1;
                    } else {
                        open_extras.push(extras);
                    }
                }
                BlockLevelElement::ContainerEnd(_) => {
                    open_extras.pop();
                }
                BlockLevelElement::Leaf(e) => {
                    let (leaf_min, leaf_max) = self.of_leaf(e);
                    let extras: f32 = open_extras.iter().sum();
                    min = min.max(leaf_min + extras);
                    max = max.max(leaf_max + extras);
                }
                BlockLevelElement::Absolute(_) => {}
            }
        }
        (min, max)
    }

    fn of_leaf(&self, entity: Entity) -> (f32, f32) {
        if let Some(ifc) = self.inline_ctxs.borrow().get(entity) {
            let breaks = &ifc.break_points;
            if breaks.len() < 2 {
                return (0.0, 0.0);
            }
            // the longest unbreakable piece and the longest line that only breaks when forced
            let mut min = 0.0f32;
            let mut max = 0.0f32;
            let mut line_start = 0;
            for i in 1..breaks.len() {
                min = min.max(ifc.content_width(i - 1, i).0);
                if breaks[i].forced || i == breaks.len() - 1 {
                    max = max.max(ifc.content_width(line_start, i).0);
                    line_start = i;
                }
            }
            (min, max)
        } else if let Some(width) = self.intrinsic_sizes.get(entity).and_then(|s| s.width()) {
            (width.0, width.0)
        } else {
            (0.0, 0.0)
        }
    }

    /// Horizontal margin, border and padding of a container, and its width if that is fixed.
    /// Percentages and `auto` count as 0 because they depend on the container.
    fn box_extras(&self, entity: Entity) -> (f32, Option<f32>) {
        let Some(style) = self.computed_values.get(entity) else {
            return (0.0, None);
        };
        let pos = &style.position;
        let absolute_length = |size: GenericSize| match size {
            GenericSize::LengthPercentage(LengthPercentage::PixelLength(px)) => Some(px.0),
            _ => None,
        };
        let extras = absolute_length(pos.margin_left).unwrap_or(0.0)
            + absolute_length(pos.margin_right).unwrap_or(0.0)
            + pos.border_left_width.0
            + pos.border_right_width.0
            + pos.padding_left.length().map_or(0.0, |l| l.0)
            + pos.padding_right.length().map_or(0.0, |l| l.0);
        (extras, absolute_length(pos.width))
    }
}

fn num(v: &Value) -> Result<f64, RuntimeError> {
    match v {
        Value::Number(n) => Ok(*n),
        _ => Err(RuntimeError::Message(format!("expected number, got {:?}", v))),
    }
}

fn inject<'world, F>(env: &EnvRef<'world>, name: &str, f: F)
where
    F: Fn(&[Value<'world>]) -> Result<Value<'world>, RuntimeError> + 'world,
{
    env.borrow_mut()
        .set(name.to_string(), Value::NativeMethod(Rc::new(f)));
}
