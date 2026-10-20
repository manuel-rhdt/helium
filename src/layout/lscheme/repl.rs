use std::rc::Rc;

use crate::layout::components::LayoutContext;


use super::{default_environment, eval, insert_components, parse_lscheme};
use rustyline;

pub fn repl(layout_context: &mut LayoutContext) -> Result<(), Box<dyn std::error::Error>> {
    let mut rl = rustyline::DefaultEditor::new()?;
    let env = default_environment()?;
    insert_components(&env, layout_context);

    loop {
        let readline = rl.readline("lscheme> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str())?;

                match parse_lscheme(line.as_str()) {
                    Ok(exprs) => {
                        for expr in exprs {
                            match eval(expr, Rc::clone(&env)) {
                                Ok(val) => println!("{:?}", val),
                                Err(e) => println!("Error: {:?}", e),
                            }
                        }
                    }
                    Err(e) => println!("Parse error: {}", e),
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted)
            | Err(rustyline::error::ReadlineError::Eof) => {
                break;
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}
