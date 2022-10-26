use std::{cell::RefCell, collections::HashMap, fmt::Debug, rc::Rc};

use env_logger::Env;
use pest::{error::Error, iterators::Pair, Parser};
use pest_derive::Parser;

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
    Ident(&'code str),
    Sexpr(Vec<Value<'code>>),
    Lambda {
        arg_names: Vec<&'code str>,
        body: Vec<Value<'code>>,
    },
    NativeFunc(for<'a> fn(&[Value<'a>]) -> Result<Value<'a>, RuntimeError>),
    // does never get constructed by parser
    Func {
        env: Environment<'code>,
        arg_names: Vec<&'code str>,
        body: Vec<Value<'code>>,
    },

    // Builtin Forms
    Define(&'code str, Box<Value<'code>>),

    // mutable values
    List(Rc<RefCell<Cons<'code>>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cons<'code> {
    car: Value<'code>,
    cdr: Option<Rc<RefCell<Cons<'code>>>>,
}

impl Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => f.debug_tuple("Null").finish(),
            Self::Number(arg0) => f.debug_tuple("Number").field(arg0).finish(),
            Self::String(arg0) => f.debug_tuple("String").field(arg0).finish(),
            Self::Ident(arg0) => f.debug_tuple("Ident").field(arg0).finish(),
            Self::List(arg0) => f.debug_tuple("List").field(arg0).finish(),
            Self::Sexpr(arg0) => f.debug_tuple("Sexpr").field(arg0).finish(),
            Self::Lambda { arg_names, body } => f
                .debug_struct("Lambda")
                .field("arg_names", arg_names)
                .field("body", body)
                .finish(),
            Self::Func {
                env,
                arg_names,
                body,
            } => f
                .debug_struct("Lambda")
                .field("env", env)
                .field("arg_names", arg_names)
                .field("body", body)
                .finish(),
            Self::NativeFunc(_) => f.debug_tuple("NativeFunc").finish(),
            Self::Define(id, val) => f.debug_tuple("Define").field(id).field(val).finish(),
        }
    }
}

impl PartialEq for Value<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Number(l0), Self::Number(r0)) => l0 == r0,
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Ident(l0), Self::Ident(r0)) => l0 == r0,
            (Self::Sexpr(l0), Self::Sexpr(r0)) => l0 == r0,
            (Self::NativeFunc(l0), Self::NativeFunc(r0)) => false,
            (Self::List(l0), Self::List(r0)) => l0 == r0,
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

pub fn parse_lscheme(program: &str) -> Result<Vec<Value>, Error<Rule>> {
    let lscheme = SchemeParser::parse(Rule::Program, program)?.next().unwrap();

    fn parse_value(pair: Pair<Rule>) -> Value {
        match pair.as_rule() {
            Rule::Number => Value::Number(pair.as_str().parse().unwrap()),
            Rule::String => Value::String(pair.into_inner().next().unwrap().as_str()),
            Rule::Ident => Value::Ident(pair.as_str()),
            Rule::Sexpr => Value::Sexpr(pair.into_inner().map(|p| parse_value(p)).collect()),
            Rule::Lambda => {
                let mut inner = pair.into_inner();
                let arg_names = parse_args(inner.next().unwrap());
                let body = inner.next().unwrap().into_inner().map(|p| parse_value(p));
                Value::Lambda {
                    arg_names: arg_names,
                    body: body.collect(),
                }
            }
            Rule::Define => {
                let mut inner = pair.into_inner();
                let id = inner.next().unwrap().as_str();
                let val = parse_value(inner.next().unwrap());
                Value::Define(id, Box::new(val))
            }
            _ => unreachable!(),
        }
    }

    Ok(lscheme.into_inner().map(|p| parse_value(p)).collect())
}

fn parse_args(pair: Pair<Rule>) -> Vec<&str> {
    pair.into_inner().map(|p| p.as_str()).collect()
}

#[derive(Debug, Clone)]
pub struct Environment<'code> {
    map: HashMap<&'code str, Value<'code>>,
    parent: Option<Box<Environment<'code>>>,
}

pub fn default_environment<'code>() -> Environment<'code> {
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

    Environment {
        map: new_map,
        parent: None,
    }
}

impl<'code> Environment<'code> {
    pub fn get(&self, k: &str) -> Option<Value<'code>> {
        self.map
            .get(k)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p_env| p_env.get(k)))
    }

    pub fn child_env(&self) -> Environment<'code> {
        Environment {
            map: HashMap::new(),
            parent: Some(Box::new(self.clone())),
        }
    }
}

pub struct CallFrame<'code> {
    returns_to: Value<'code>,
    environment: Environment<'code>,
    evaluated_arguments: Vec<Value<'code>>,
    previous_frame: Option<Box<CallFrame<'code>>>,
}

fn push_frame<'code>(
    current_frame: &mut Option<CallFrame<'code>>,
    returns_to: Value<'code>,
    env: &mut Environment<'code>,
    argument_rib: &mut Vec<Value<'code>>,
) {
    let new_env = env.child_env();
    let old_env = std::mem::replace(env, new_env);
    let new_frame = CallFrame {
        returns_to: returns_to,
        environment: old_env,
        evaluated_arguments: argument_rib.clone(),
        previous_frame: None,
    };
    let previous_frame = std::mem::replace(current_frame, Some(new_frame));
    current_frame.as_mut().unwrap().previous_frame = previous_frame.map(Box::new);
    argument_rib.clear();
}

fn pop_frame<'code>(
    current_frame: &mut Option<CallFrame<'code>>,
    env: &mut Environment<'code>,
    argument_rib: &mut Vec<Value<'code>>,
) {
    if let Some(frame) = current_frame.take() {
        *argument_rib = frame.evaluated_arguments;
        *env = frame.environment;
        *current_frame = frame.previous_frame.map(|f| *f);
    } else {
        panic!("Tried to pop empty frame")
    }
}

pub fn eval<'code>(
    mut next_expression: Value<'code>,
    mut current_frame: Option<CallFrame<'code>>,
    env: &mut Environment<'code>,
) -> Result<Value<'code>, RuntimeError> {
    let mut argument_rib = vec![];
    let mut accumulator;

    loop {
        match &next_expression {
            Value::Define(id, val) => {
                push_frame(
                    &mut current_frame,
                    next_expression.clone(),
                    env,
                    &mut argument_rib,
                );
                next_expression = val.as_ref().clone();
                continue;
            }
            Value::Sexpr(list) => {
                push_frame(
                    &mut current_frame,
                    next_expression.clone(),
                    env,
                    &mut argument_rib,
                );
                next_expression = list[0].clone(); // in normal condition never panics because Sexpr should not be empty
                continue;
            }
            Value::Lambda { arg_names, body } => {
                // construct a bound function
                accumulator = Some(Value::Func {
                    env: env.clone(),
                    arg_names: arg_names.clone(),
                    body: body.clone(),
                })
            }
            Value::Ident(name) => {
                let val = env.get(name).ok_or(RuntimeError::Message(format!(
                    "Undefined identifier: {:?}",
                    name
                )))?;
                accumulator = Some(val);
            }
            other => accumulator = Some(other.clone()),
        };

        assert!(accumulator.is_some());

        // determine next expression to evaluate

        loop {
            match current_frame.as_ref().map(|f| &f.returns_to) {
                Some(Value::Define(id, _val)) => {
                    let id: &'code str = id;
                    pop_frame(&mut current_frame, env, &mut argument_rib);
                    // we have already evaluated value and stored it in the accumulator
                    env.map.insert(id, accumulator.take().unwrap());
                    // define does not return a value
                    accumulator = Some(Value::Null);
                }
                Some(Value::Sexpr(vec)) => {
                    // first we just need to evaluate all arguments

                    let num_args = vec.len();
                    let is_evaluated = argument_rib.len() == num_args;

                    if !is_evaluated {
                        // the accumulator contains the last evaluated argument
                        argument_rib.push(accumulator.take().unwrap());
                    }

                    if argument_rib.len() < num_args {
                        // evaluate next argument
                        next_expression = vec[argument_rib.len()].clone();
                        break;
                    }
                    // once all arguments were evaluated we replace the current Sexpr with the function body
                    else if !is_evaluated {
                        // evaluate function application
                        let func = &argument_rib[0];
                        let evaluated_args = &argument_rib[1..];
                        let result = eval_fn(func, evaluated_args, env)?;
                        next_expression = result;
                        break;
                    } else {
                        // the function evaluation finished with the return value stored in accumulator
                        pop_frame(&mut current_frame, env, &mut argument_rib);
                    }
                }
                Some(_) => break,
                None => return Ok(accumulator.unwrap()),
            }
        }

        assert!(accumulator.is_none());
    }
}

pub fn parse_and_eval(program: &str) -> Option<Value> {
    let parsed = parse_lscheme(program).unwrap();
    let mut env = default_environment();
    let mut result = None;
    for val in parsed {
        result = Some(eval(val, None, &mut env).unwrap());
    }
    result
}

// mutates the environment and returns an expression to evaluate
fn eval_fn<'code>(
    func: &Value<'code>,
    args: &[Value<'code>],
    env: &mut Environment<'code>,
) -> Result<Value<'code>, RuntimeError> {
    // 1. form a new environment by combining the current environment with the function arguments
    // 2. return the expression to be evaluated as the current expression

    match func {
        Value::NativeFunc(f) => f(args),
        Value::Func {
            env: lambda_env,
            arg_names,
            body,
        } => {
            *env = lambda_env.clone();
            for (&arg_name, evaluated_arg) in arg_names.iter().zip(args) {
                env.map.insert(arg_name, evaluated_arg.clone());
            }
            Ok(body[0].clone())
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
    fn check_simple() {
        let result = parse_and_eval("(+ 3 (- (+ 3 4) 4))").unwrap();
        assert_eq!(result, Value::Number(6.0));
    }

    #[test]
    fn check_lambda() {
        let result = parse_and_eval("((lambda (a b) (+ a b)) 3 4)").unwrap();
        assert_eq!(result, Value::Number(7.0));
    }

    #[test]
    fn check_define() {
        let result = parse_and_eval("(define a 5) a").unwrap();
        assert_eq!(result, Value::Number(5.0));

        let result = parse_and_eval("(define a (+ 1 2)) a").unwrap();
        assert_eq!(result, Value::Number(3.0));

        let result =
            parse_and_eval("(define my_add (lambda (a b) (+ a b)))\n(my_add (+ 3 4) 5)").unwrap();
        assert_eq!(result, Value::Number(3.0 + 4.0 + 5.0));
    }
}
