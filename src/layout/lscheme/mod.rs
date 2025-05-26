use std::{cell::RefCell, collections::HashMap, fmt::Debug, rc::Rc};

use env_logger::Env;
use font_parse::tables::head;
use html5ever::tendril::fmt::imp;
use pest::{error::Error, iterators::Pair, Parser};
use pest_derive::Parser;
use specs::rayon::result;

#[derive(Parser)]
#[grammar = "layout/lscheme/grammar.pest"]
struct SchemeParser;

#[derive(Debug)]
pub enum RuntimeError {
    Message(String),
}

#[derive(Clone)]
pub enum Value<'code> {
    // immutable values
    Null,
    Number(f64),
    String(&'code str),
    Symbol(&'code str),
    List(Vec<Value<'code>>),

    // The following values are created by the evaluator
    NativeFunc(for<'a> fn(&[Value<'a>]) -> Result<Value<'a>, RuntimeError>),

    // Represents a function including its environment.
    Closure {
        env: EnvRef<'code>,
        arg_names: Vec<&'code str>,
        body: Vec<Value<'code>>,
    },

    // Builtin Special Forms
    Define(&'code str, Box<Value<'code>>),
    DefineSyntax(&'code str, Box<Value<'code>>),
}

impl Value<'_> {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Self::Number(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    pub fn is_symbol(&self) -> bool {
        matches!(self, Self::Symbol(_))
    }

    pub fn is_list(&self) -> bool {
        matches!(self, Self::List(_))
    }
}

impl Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => f.debug_tuple("Null").finish(),
            Self::Number(arg0) => write!(f, "{}", arg0),
            Self::String(arg0) => write!(f, "\"{}\"", arg0),
            Self::Symbol(arg0) => write!(f, "{}", arg0),
            Self::List(arg0) => {
                write!(f, "(")?;
                for item in arg0 {
                    write!(f, "{:?} ", item)?;
                }
                write!(f, ")")
            }
            Self::Closure {
                env,
                arg_names,
                body,
            } => f
                .debug_struct("Closure")
                .field("env", env)
                .field("arg_names", arg_names)
                .field("body", body)
                .finish(),
            Self::NativeFunc(_) => f.debug_tuple("NativeFunc").finish(),
            Self::Define(id, val) => f.debug_tuple("Define").field(id).field(val).finish(),
            Self::DefineSyntax(id, val) => {
                f.debug_tuple("DefineSyntax").field(id).field(val).finish()
            }
        }
    }
}

impl PartialEq for Value<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Number(l0), Self::Number(r0)) => l0 == r0,
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Symbol(l0), Self::Symbol(r0)) => l0 == r0,
            (Self::List(l0), Self::List(r0)) => l0 == r0,
            (Self::NativeFunc(l0), Self::NativeFunc(r0)) => false,
            _ => false,
        }
    }
}

fn expect_number(val: &Value) -> Result<f64, RuntimeError> {
    match val {
        Value::Number(num) => Ok(*num),
        _ => Err(RuntimeError::Message("expected number".to_string())),
    }
}

pub fn parse_lscheme(program: &str) -> Result<Vec<Value<'_>>, Error<Rule>> {
    let lscheme = SchemeParser::parse(Rule::Program, program)?.next().unwrap();

    fn parse_value(pair: Pair<Rule>) -> Value<'_> {
        match pair.as_rule() {
            Rule::Number => Value::Number(pair.as_str().parse().unwrap()),
            Rule::String => Value::String(pair.into_inner().next().unwrap().as_str()),
            Rule::Ident => Value::Symbol(pair.as_str()),
            Rule::List => Value::List(pair.into_inner().map(|p| parse_value(p)).collect()),
            other => panic!("Unexpected rule: {:?}", other),
        }
    }

    Ok(lscheme.into_inner().map(|p| parse_value(p)).collect::<Vec<_>>())
}

#[derive(Debug, Clone)]
pub struct Environment<'code> {
    map: HashMap<&'code str, Value<'code>>,
    parent: Option<EnvRef<'code>>,
}

pub fn default_environment<'code>() -> EnvRef<'code> {
    let mut new_map = HashMap::new();

    fn plus<'code>(args: &[Value<'code>]) -> Result<Value<'code>, RuntimeError> {
        args.iter().try_fold(Value::Number(0.0), |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a + b))
        })
    }
    new_map.insert("+", Value::NativeFunc(plus));

    fn minus<'code>(args: &[Value<'code>]) -> Result<Value<'code>, RuntimeError> {
        let first_arg = args
            .get(0)
            .ok_or(RuntimeError::Message(format!("called - without arguments")))?
            .clone();
        (&args[1..]).iter().try_fold(first_arg, |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a - b))
        })
    }
    new_map.insert("-", Value::NativeFunc(minus));

    Rc::new(RefCell::new(Environment {
        map: new_map,
        parent: None,
    }))
}

type EnvRef<'code> = Rc<RefCell<Environment<'code>>>;

impl<'code> Environment<'code> {
    pub fn get(&self, k: &str) -> Option<Value<'code>> {
        self.map
            .get(k)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p_env| p_env.borrow().get(k)))
    }

    pub fn child_env(&self) -> Environment<'code> {
        Environment {
            map: HashMap::new(),
            parent: Some(Rc::new(RefCell::new(self.clone()))),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Continuation<'code> {
    ApplyFunction {
        proc: Option<Value<'code>>,
        args: Vec<Value<'code>>,
        evaluated_args: Vec<Value<'code>>,
        env: EnvRef<'code>,
    },
    EvaluateDefine {
        name: &'code str,
        env: EnvRef<'code>,
    },
}

pub struct CallStack<'code> {
    stack: Vec<Continuation<'code>>,
}

impl<'code> CallStack<'code> {
    pub fn push_frame(&mut self, continuation: Continuation<'code>) {
        self.stack.push(continuation);
    }

    pub fn pop_frame(&mut self) -> Continuation<'code> {
        self.stack.pop().unwrap()
    }
}

#[derive(Debug)]
enum Thunk<'code> {
    Value(Value<'code>),
    Return(Value<'code>),
    Eval(Value<'code>, EnvRef<'code>),
    TailCall(Value<'code>, EnvRef<'code>),
}

impl<'code> Continuation<'code> {
    fn apply_value(&mut self, val: Value<'code>) -> Result<Thunk<'code>, RuntimeError> {
        match self {
            Continuation::EvaluateDefine { name, env } => {
                // In this case, `val` is the value to be defined
                env.borrow_mut().map.insert(name, val);
                Ok(Thunk::Return(Value::Null)) // Return null after defining
            }
            Continuation::ApplyFunction {
                proc,
                args,
                evaluated_args,
                env,
                ..
            } => {
                if proc.is_none() {
                    // `val` represents the evaluated procedure (head of the list)
                    *proc = Some(val);

                    if args.is_empty() {
                        // If there are no arguments, we can apply the function immediately
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(&func, &[])?;
                        Ok(val)
                    } else {
                        // Otherwise, we continue evaluating the arguments
                        let first_arg = args[0].clone();
                        Ok(Thunk::Eval(first_arg, Rc::clone(env)))
                    }
                } else {
                    if evaluated_args.len() >= args.len() {
                        panic!("Called continuation after it already is exhausted. This is a bug in the interpreter.");
                    }
                    evaluated_args.push(val);
                    if evaluated_args.len() == args.len() {
                        // All arguments are evaluated, we can apply the function
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(func, evaluated_args)?;
                        Ok(val)
                    } else {
                        // Continue evaluating the next argument
                        let next_arg = args[evaluated_args.len()].clone();
                        Ok(Thunk::Eval(next_arg, Rc::clone(env)))
                    }
                }
            }
        }
    }
}

fn trampoline<'code>(thunk: Thunk<'code>) -> Result<Value<'code>, RuntimeError> {
    let mut current_thunk = thunk;
    let mut stack = CallStack { stack: vec![] };
    let mut counter = 0;
    loop {
        if counter > 1000 {
            return Err(RuntimeError::Message("Stack overflow".to_string()));
        } else {
            counter += 1;
        }
        match current_thunk {
            Thunk::Value(val) => {
                if let Some(frame) = stack.stack.last_mut() {
                    current_thunk = frame.apply_value(val)?;
                } else {
                    return Ok(val);
                }
            }
            Thunk::Return(val) => {
                stack.pop_frame();
                current_thunk = Thunk::Value(val);
            }
            Thunk::Eval(expr, env) => {
                current_thunk = eval_expr(expr, env, &mut stack)?;
            }
            Thunk::TailCall(expr, env) => {
                stack.pop_frame();
                current_thunk = Thunk::Eval(expr, env);
            }
        }
    }
}

fn eval_expr<'code>(
    expr: Value<'code>,
    env: EnvRef<'code>,
    stack: &mut CallStack<'code>,
) -> Result<Thunk<'code>, RuntimeError> {
    match expr {
        Value::Symbol(name) => {
            if let Some(val) = env.borrow().get(name) {
                Ok(Thunk::Value(val))
            } else {
                Err(RuntimeError::Message(format!(
                    "Undefined identifier: {:?}",
                    name
                )))
            }
        }
        Value::List(list) => {
            if list.is_empty() {
                return Err(RuntimeError::Message(format!(
                    "Empty list cannot be evaluated"
                )));
            }

            let head = &list[0];

            match head {
                Value::Symbol("define") => {
                    if list.len() != 3 {
                        return Err(RuntimeError::Message(
                            "define requires exactly two arguments".to_string(),
                        ));
                    }
                    let name = match &list[1] {
                        Value::Symbol(s) => *s,
                        _ => {
                            return Err(RuntimeError::Message(
                                "define first argument must be a symbol".to_string(),
                            ))
                        }
                    };
                    let value = &list[2];
                    stack.push_frame(Continuation::EvaluateDefine {
                        name: name,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(value.clone(), Rc::clone(&env)))
                }
                Value::Symbol("lambda") => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "lambda requires at least two arguments".to_string(),
                        ));
                    }
                    let arg_names = match &list[1] {
                        Value::List(args) => args
                            .iter()
                            .map(|v| match v {
                                Value::Symbol(s) => Ok(*s),
                                _ => Err(RuntimeError::Message(format!(
                                    "lambda argument must be a symbol"
                                ))),
                            })
                            .collect::<Result<_, _>>()?,
                        _ => {
                            return Err(RuntimeError::Message(
                                "lambda second argument must be a list of symbols".to_string(),
                            ))
                        }
                    };
                    let body = list[2..].to_vec();
                    let closure = Value::Closure {
                        env: Rc::new(env.as_ref().clone()),
                        arg_names,
                        body,
                    };
                    Ok(Thunk::Value(closure))
                }
                head => {
                    // create a new continuation
                    stack.push_frame(Continuation::ApplyFunction {
                        proc: None,
                        args: list[1..].to_vec(),
                        evaluated_args: vec![],
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(head.clone(), env))
                }
            }
        }
        other => Ok(Thunk::Value(other)),
    }
}

pub fn eval<'code>(expr: Value<'code>, env: EnvRef<'code>) -> Result<Value<'code>, RuntimeError> {
    trampoline(Thunk::Eval(expr, env))
}

pub fn parse_and_eval(program: &str) -> Option<Value> {
    let parsed = parse_lscheme(program).unwrap();
    let env = default_environment();
    let mut result = None;
    for val in parsed {
        result = Some(eval(val, Rc::clone(&env)).unwrap());
    }
    result
}

// mutates the environment and returns an expression to evaluate
fn eval_fn<'code>(
    func: &Value<'code>,
    args: &[Value<'code>],
) -> Result<Thunk<'code>, RuntimeError> {
    // 1. form a new environment by combining the current environment with the function arguments
    // 2. return the expression to be evaluated as the current expression

    match func {
        Value::NativeFunc(f) => Ok(Thunk::Return(f(args)?)),
        Value::Closure {
            env: lambda_env,
            arg_names,
            body,
        } => {
            let mut env = lambda_env.borrow().child_env();
            for (&arg_name, evaluated_arg) in arg_names.iter().zip(args) {
                env.map.insert(arg_name, evaluated_arg.clone());
            }
            Ok(Thunk::TailCall(body[0].clone(), Rc::new(env.into())))
        }
        val => Err(RuntimeError::Message(format!(
            "Expected function, got {:?}",
            val
        ))),
    }
}

mod test {
    use super::*;

    #[test]
    fn test_parse_number() {
        let program = "123";
        let expected = Ok(vec![Value::Number(123.0)]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_negative_number() {
        let program = "-45";
        let expected = Ok(vec![Value::Number(-45.0)]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_string() {
        let program = r#""hello""#;
        let expected = Ok(vec![Value::String("hello")]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_symbol() {
        let program = "foo";
        let expected = Ok(vec![Value::Symbol("foo")]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_simple_list() {
        let program = "(+ 1 2)";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("+"),
            Value::Number(1.0),
            Value::Number(2.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_nested_list() {
        let program = "(define x (list 1 2))";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("define"),
            Value::Symbol("x"),
            Value::List(vec![
                Value::Symbol("list"),
                Value::Number(1.0),
                Value::Number(2.0),
            ]),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_multiple_top_level_expressions() {
        let program = "(+ 1 2) (define a 10)";
        let expected = Ok(vec![
            Value::List(vec![
                Value::Symbol("+"),
                Value::Number(1.0),
                Value::Number(2.0),
            ]),
            Value::List(vec![
                Value::Symbol("define"),
                Value::Symbol("a"),
                Value::Number(10.0),
            ]),
        ]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_empty_list() {
        let program = "()";
        let expected = Ok(vec![Value::List(vec![])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_list_with_mixed_types() {
        let program = r#"(set! my-var "hello" 123)"#;
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("set!"),
            Value::Symbol("my-var"),
            Value::String("hello"),
            Value::Number(123.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let program = "  ( +   1   2   )  ";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("+"),
            Value::Number(1.0),
            Value::Number(2.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_nested_application() {
        let result = parse_and_eval("(+ 3 (- (+ 13 7) 4))").unwrap();
        assert_eq!(result, Value::Number(19.0));
    }

    #[test]
    fn test_lambda() {
        let result = parse_and_eval("((lambda (a b) (+ a b)) 3 4)").unwrap();
        assert_eq!(result, Value::Number(7.0));

        let result = parse_and_eval("((lambda () (+ 3 4)))").unwrap();
        assert_eq!(result, Value::Number(7.0));

        let result = parse_and_eval("(((lambda (a) (lambda (b) (+ a b))) 3) 4)").unwrap();
        assert_eq!(result, Value::Number(7.0));
    }

    #[test]
    fn test_define() {
        let result = parse_and_eval("(define a 5) a").unwrap();
        assert_eq!(result, Value::Number(5.0));

        let result = parse_and_eval("(define a (+ 1 2)) a").unwrap();
        assert_eq!(result, Value::Number(3.0));

        // define returns NULL
        let result =
            parse_and_eval("(define my_add (lambda (a b) (+ a b)))").unwrap();
        assert!(result.is_null());

        let result =
            parse_and_eval("(define my_add (lambda (a b) (+ a b)))\n(my_add (+ 3 4) 5)").unwrap();
        assert_eq!(result, Value::Number(3.0 + 4.0 + 5.0));
    }
}
