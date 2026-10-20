use std::rc::Rc;

use specs::{Entity, World};

#[allow(unused_imports)]
use crate::layout::components::{EdgeValues, Position, TextList, ZIndex};
use crate::style::properties::computed::PixelLength;

use super::{
    default_environment, eval, parse_lscheme, RuntimeError,
    ecs_binding::setup_ecs_env,
};

static BLOCK_LAYOUT_SCM: &str = include_str!("block_layout.scm");

pub struct SchemeBlockLayout {
    pub root_entity: Entity,
    pub page_width: PixelLength,
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

impl SchemeBlockLayout {
    pub fn run(self, world: &mut World) -> Result<(), RuntimeError> {
        run_scheme_block_layout(
            self.root_entity,
            self.page_width,
            self.page_height,
            self.margin,
            world,
        )
    }
}

fn run_scheme_block_layout(
    root_entity: Entity,
    page_width: PixelLength,
    page_height: PixelLength,
    margin: EdgeValues<PixelLength>,
    world: &mut World,
) -> Result<(), RuntimeError> {
    let env = default_environment()?;
    setup_ecs_env(&env, world);

    // Run the layout in a closure so the environment is cleared on both success and error.
    let result = (|| {
        // Define run-block-layout
        for form in parse_lscheme(BLOCK_LAYOUT_SCM)? {
            eval(form, Rc::clone(&env))?;
        }

        // Call: (run-block-layout root-id page-width page-height margin-top margin-right
        //                         margin-bottom margin-left)
        use super::Value;
        let call = Value::List(vec![
            Value::Symbol("run-block-layout".to_string()),
            // quoted, otherwise the (id generation) list would be evaluated as a call
            Value::List(vec![Value::Symbol("quote".to_string()), Value::from(root_entity)]),
            Value::Number(page_width.0 as f64),
            Value::Number(page_height.0 as f64),
            Value::Number(margin.top.0 as f64),
            Value::Number(margin.right.0 as f64),
            Value::Number(margin.bottom.0 as f64),
            Value::Number(margin.left.0 as f64),
        ]);
        eval(call, Rc::clone(&env)).map(|_| ())
    })();

    // Break the Rc cycle (closures in env hold Rc<env>) so WriteStorage borrows are released
    // before the world is used again in tests or subsequent systems.
    env.borrow_mut().clear_bindings();
    result
}

// ---------------------------------------------------------------------------
// Tests — mirror the block layout tests in normal_formatting.rs
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::TryInto;

    use specs::prelude::*;
    use crate::simple_dom::{body, div, html, Quantity::*};

    struct SchemeLayoutTest<'a> {
        html: &'a crate::simple_dom::Node,
        page_width: crate::simple_dom::Quantity,
    }

    impl<'a> SchemeLayoutTest<'a> {
        fn layout(self) -> specs::World {
            let mut world = self.html.construct();
            let page_width: PixelLength = self.page_width.try_into().unwrap();

            world.insert(TextList::default());

            let root_entity = self.html.entity();
            run_scheme_block_layout(
                root_entity,
                page_width,
                PixelLength(1000.0),
                EdgeValues::all(PixelLength(0.0)),
                &mut world,
            )
            .expect("scheme block layout failed");

            world
        }
    }

    #[test]
    fn test_scheme_vertical_stacking() {
        let html = &html(vec![
            div(vec![])
                .height(Px(10.0))
                .margin_top(Px(10.0))
                .margin_bottom(Px(30.0)),
            div(vec![]),
        ]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(pos1.y, PixelLength(10.0));
        assert_eq!(pos2.y, PixelLength(50.0));
    }

    #[test]
    fn test_scheme_content_height_of_container_with_children() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)),
            div(vec![]).height(Px(40.0)),
        ]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos: &Position = html.get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(100.0),
                height: PixelLength(50.0),
                z: ZIndex(1),
                page: 0,
            }
        );
    }

    const EPSILON: PixelLength = PixelLength(1e-5);

    #[test]
    fn test_scheme_nested_elements() {
        let child = div(vec![]).height(Px(20.0)).margin_bottom(Px(30.0));
        let html = &html(vec![body(vec![div(vec![child.clone(), child])
            .padding(Px(10.0))
            .margin_top(Px(5.0))])
        .margin_left(Px(10.0))]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let container_pos: &Position = html["body"][0].get(&positions);

        assert_eq!(
            (container_pos.x, container_pos.y),
            (PixelLength(20.0), PixelLength(15.0))
        );

        let child1_pos = html["body"][0][0].get(&positions);
        assert!((child1_pos.x - container_pos.x).abs() <= EPSILON);
        assert!((child1_pos.y - container_pos.y).abs() <= EPSILON);
        assert!(child1_pos.height == PixelLength(20.0));

        let child2_pos = html["body"][0][1].get(&positions);
        assert!((child2_pos.x - container_pos.x).abs() <= EPSILON);
        assert!((child2_pos.y - child1_pos.y - PixelLength(50.0)).abs() <= EPSILON);
        assert!(child2_pos.height == PixelLength(20.0));
    }

    #[test]
    fn test_scheme_left_margin() {
        let html = &html(vec![
            div(vec![div(vec![]).margin_left(Px(15.0))]).margin_left(Px(20.0))
        ]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos: &Position = html["div"].get(&positions);
        let pos_nested: &Position = html["div"]["div"].get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(20.0),
                y: PixelLength(0.0),
                width: PixelLength(80.0),
                height: PixelLength(0.0),
                z: ZIndex(2),
                page: 0,
            }
        );

        assert_eq!(
            pos_nested,
            &Position {
                x: PixelLength(35.0),
                y: PixelLength(0.0),
                width: PixelLength(65.0),
                height: PixelLength(0.0),
                z: ZIndex(3),
                page: 0,
            }
        );
    }

    #[test]
    fn test_scheme_margin_top_offsets_first_element() {
        let html = &html(vec![div(vec![]).margin_top(Px(10.0))]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos: &Position = html[0].get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(10.0),
                width: PixelLength(100.0),
                height: PixelLength(0.0),
                z: ZIndex(2),
                page: 0,
            }
        );
    }

    #[test]
    fn test_scheme_margin_collapsing() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)).margin_bottom(Px(30.0)),
            div(vec![]).height(Px(10.0)).margin_top(Px(10.0)),
        ]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(pos2.y, PixelLength(40.0));
    }

    #[test]
    fn test_scheme_percentage_width() {
        let html = &html(vec![
            div(vec![]).width(Percent(50.0)),
            div(vec![]).width(Percent(150.0)),
        ]);
        let world = SchemeLayoutTest { html, page_width: Px(100.0) }.layout();

        let positions = world.read_storage();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(
            pos1,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(50.0),
                height: PixelLength(0.0),
                z: ZIndex(2),
                page: 0,
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(150.0),
                height: PixelLength(0.0),
                z: ZIndex(2),
                page: 0,
            }
        );
    }

    fn eval_in_layout_env(world: &mut World, program: &str) -> Result<(), RuntimeError> {
        let env = default_environment()?;
        setup_ecs_env(&env, world);
        let mut result = Ok(());
        for form in parse_lscheme(program)? {
            result = eval(form, Rc::clone(&env)).map(|_| ());
            if result.is_err() {
                break;
            }
        }
        env.borrow_mut().clear_bindings();
        result
    }

    #[test]
    fn test_stale_entity_generation_is_error() {
        let html = html(vec![div(vec![])]);
        let mut world = html.construct();
        world.insert(TextList::default());
        let ent = html.entity();
        let call = |generation: i32| {
            format!("(set-position! (list {} {}) 0 0 1 1 0 0)", ent.id(), generation)
        };
        assert!(eval_in_layout_env(&mut world, &call(ent.r#gen().id())).is_ok());
        assert!(eval_in_layout_env(&mut world, &call(ent.r#gen().id() + 1)).is_err());
    }

    #[test]
    fn test_layout_inline_lines_errors() {
        let html = html(vec![div(vec![])]);
        let mut world = html.construct();
        world.insert(TextList::default());
        // wrong number of arguments
        assert!(eval_in_layout_env(&mut world, "(layout-inline-lines (list 1 1) 100.0)").is_err());
        // unknown entity
        assert!(eval_in_layout_env(&mut world, "(layout-inline-lines (list 999999 1) 100.0 0.0)").is_err());
        // entity without inline context
        let ent = html.entity();
        let call = format!("(layout-inline-lines (list {} {}) 100.0 0.0)", ent.id(), ent.r#gen().id());
        assert!(eval_in_layout_env(&mut world, &call).is_err());

    }
}
