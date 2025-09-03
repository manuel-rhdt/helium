use std::{cell::RefCell, collections::HashMap, fmt::Debug, rc::Rc};

use font_parse::TableRecord;
use pest::{Parser, error::Error, iterators::Pair};
use pest_derive::Parser;

use specs::{
    prelude::{Entities, Join, ReadStorage, SystemData}, shred::{self, Fetch}, Entity, Read
};

mod repl;

use crate::style::properties::computed::ComputedValues;

pub use self::repl::repl;

use super::components::{LayoutContext, RootNode};

#[derive(Parser)]
#[grammar = "layout/lscheme/grammar.pest"]
struct SchemeParser;

#[derive(Debug)]
pub enum RuntimeError {
    Message(String),
}

macro_rules! resources {
    ($name:ident<$lt:lifetime>($($component_name:ident: $component_type:ty),+)) => {
        pub enum $name<$lt> {
            $($component_name($component_type)),+
        }

        impl Debug for $name<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $(Self::$component_name(_) => {
                        f.debug_tuple(stringify!($name))
                            .field(&stringify!($component_name))
                            .finish()
                    }),+
                }
            }
        }

        impl<'lt> $name<'lt> {
            pub fn get_value(&self) -> Value<'lt> {
                match self {
                    $(Self::$component_name(resource) => {
                        resource.get_value().unwrap_or(Value::Null)
                    }),+
                }
            }
        }

        pub fn insert_components<'world>(
            env: &EnvRef<'world>,
            world: &'world mut specs::World) {

            let mut env_mut = env.borrow_mut();
            $(
                let val = Value::Ecs(Rc::new(Resource::$component_name(
                    <$component_type>::fetch(world)
                )));
                env_mut.set(stringify!($component_name).to_string(), val);
            )+
            env_mut.set("get".to_string(), Value::NativeFunc(|args| {
                if args.len() != 1 {
                    return Err(RuntimeError::Message("get expects one argument".to_string()));
                }
                match &args[0] {
                    Value::Ecs(components) => Ok(components.get_value()),
                    _ => Err(RuntimeError::Message("get expects an ECS component".to_string())),
                }
            }));
        }
    };
}

trait GetValue<'world>
where
    Self: Sized,
{
    fn get_value(&self) -> Result<Value<'world>, RuntimeError>;
}

impl<'world, T: shred::Resource> GetValue<'world> for Read<'world, T>
where
    for<'a> Value<'world>: From<&'a T>,
{
    fn get_value(&self) -> Result<Value<'world>, RuntimeError> {
        let value: &T = &self;
        let value = value.into();
        Ok(value)
    }
}

impl<'world> GetValue<'world> for Entities<'world> {
    fn get_value(&self) -> Result<Value<'world>, RuntimeError> {
        let value = self.join().map(Into::into).collect::<Vec<_>>();
        Ok(Value::List(value))
    }
}

impl<'world, T: specs::Component> GetValue<'world> for ReadStorage<'world, T>
where
    for<'a> Value<'world>: From<&'a T>,
{
    fn get_value(&self) -> Result<Value<'world>, RuntimeError> {
        let value = self.join().map(Into::into).collect::<Vec<_>>();
        Ok(Value::List(value))
    }
}

impl From<Entity> for Value<'_> {
    fn from(ent: Entity) -> Self {
        Value::List(vec![
            Value::Number(ent.id() as f64),
            Value::Number(ent.r#gen().id() as f64),
        ])
    }
}

impl From<&ComputedValues> for Value<'_> {
    fn from(value: &ComputedValues) -> Self {
        Value::String(format!("{:?}", value))
    }
}

impl From<&RootNode> for Value<'_> {
    fn from(value: &RootNode) -> Self {
        match value {
            RootNode(Some(ent)) => (*ent).into(),
            RootNode(None) => Value::Null,
        }
    }
}

resources!(Resource<'world>(
    ComputedValues: ReadStorage<'world, ComputedValues>,
    Entities: Entities<'world>,
    Root: Read<'world, RootNode>)
);

#[derive(Clone)]
pub enum Value<'world> {
    // immutable values
    Null,
    Number(f64),
    String(String),
    Symbol(String),
    List(Vec<Value<'world>>),

    // The following values are created by the evaluator
    NativeFunc(for<'a> fn(&[Value<'a>]) -> Result<Value<'a>, RuntimeError>),

    // Represents a function including its environment.
    Closure {
        env: EnvRef<'world>,
        arg_names: Vec<String>,
        body: Vec<Value<'world>>,
    },

    // Native types
    Ecs(Rc<Resource<'world>>),
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
            Self::Ecs(c) => write!(f, "{:?}", c),
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

pub fn parse_lscheme<'world>(program: &str) -> Result<Vec<Value<'world>>, Error<Rule>> {
    let lscheme = SchemeParser::parse(Rule::Program, program)?.next().unwrap();

    fn parse_value<'a>(pair: Pair<Rule>) -> Value<'a> {
        match pair.as_rule() {
            Rule::Number => Value::Number(pair.as_str().parse().unwrap()),
            Rule::String => Value::String(pair.into_inner().next().unwrap().as_str().into()),
            Rule::Ident => Value::Symbol(pair.as_str().into()),
            Rule::List => Value::List(pair.into_inner().map(|p| parse_value(p)).collect()),
            other => panic!("Unexpected rule: {:?}", other),
        }
    }

    Ok(lscheme
        .into_inner()
        .map(|p| parse_value(p))
        .collect::<Vec<Value<'world>>>())
}

#[derive(Debug, Clone)]
pub struct Environment<'world> {
    map: HashMap<String, Value<'world>>,
    parent: Option<EnvRef<'world>>,
}

pub fn default_environment<'a>() -> EnvRef<'a> {
    let mut new_map = HashMap::new();

    fn plus<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        args.iter().try_fold(Value::Number(0.0), |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a + b))
        })
    }
    new_map.insert("+".to_string(), Value::NativeFunc(plus));

    fn minus<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
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
    new_map.insert("-".to_string(), Value::NativeFunc(minus));

    fn length<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message(
                "length expects one argument".to_string(),
            ));
        }
        match &args[0] {
            Value::List(list) => Ok(Value::Number(list.len() as f64)),
            _ => Err(RuntimeError::Message("length expects a list".to_string())),
        }
    }
    new_map.insert("length".to_string(), Value::NativeFunc(length));

    Rc::new(RefCell::new(Environment {
        map: new_map,
        parent: None,
    }))
}

type EnvRef<'world> = Rc<RefCell<Environment<'world>>>;

impl<'world> Environment<'world> {
    pub fn set(&mut self, k: String, v: Value<'world>) {
        self.map.insert(k, v);
    }

    pub fn get(&self, k: &str) -> Option<Value<'world>> {
        self.map
            .get(k)
            .cloned()
            .or_else(|| self.parent.as_ref().and_then(|p_env| p_env.borrow().get(k)))
    }

    pub fn child_env(&self) -> Self {
        Environment {
            map: HashMap::new(),
            parent: Some(Rc::new(RefCell::new(self.clone()))),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Continuation<'world> {
    ApplyFunction {
        proc: Option<Value<'world>>,
        args: Vec<Value<'world>>,
        evaluated_args: Vec<Value<'world>>,
        env: EnvRef<'world>,
    },
    EvaluateDefine {
        name: String,
        env: EnvRef<'world>,
    },
}

#[derive(Debug, Clone)]
pub struct CallStack<'world> {
    stack: Vec<Continuation<'world>>,
}

impl<'world> CallStack<'world> {
    pub fn push_frame(&mut self, continuation: Continuation<'world>) {
        self.stack.push(continuation);
    }

    pub fn pop_frame(&mut self) -> Continuation<'world> {
        self.stack.pop().unwrap()
    }
}

#[derive(Debug)]
enum Thunk<'world> {
    Value(Value<'world>),
    Return(Value<'world>),
    Eval(Value<'world>, EnvRef<'world>),
    TailCall(Value<'world>, EnvRef<'world>),
}

impl<'world> Continuation<'world> {
    fn apply_value(&mut self, val: Value<'world>) -> Result<Thunk<'world>, RuntimeError> {
        match self {
            Continuation::EvaluateDefine { name, env } => {
                // In this case, `val` is the value to be defined
                env.borrow_mut().map.insert(name.clone(), val);
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
                        panic!(
                            "Called continuation after it already is exhausted. This is a bug in the interpreter."
                        );
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

fn trampoline(thunk: Thunk<'_>) -> Result<Value<'_>, RuntimeError> {
    let mut current_thunk = thunk;
    let mut stack = CallStack { stack: vec![] };
    loop {
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

fn eval_expr<'world>(
    expr: Value<'world>,
    env: EnvRef<'world>,
    stack: &mut CallStack<'world>,
) -> Result<Thunk<'world>, RuntimeError> {
    match expr {
        Value::Symbol(name) => {
            if let Some(val) = env.borrow().get(&name) {
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
                Value::Symbol(x) if x == "define" => {
                    if list.len() != 3 {
                        return Err(RuntimeError::Message(
                            "define requires exactly two arguments".to_string(),
                        ));
                    }
                    let name = match &list[1] {
                        Value::Symbol(s) => s.clone(),
                        _ => {
                            return Err(RuntimeError::Message(
                                "define first argument must be a symbol".to_string(),
                            ));
                        }
                    };
                    let value = &list[2];
                    stack.push_frame(Continuation::EvaluateDefine {
                        name: name,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(value.clone(), Rc::clone(&env)))
                }
                Value::Symbol(x) if x == "lambda" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "lambda requires at least two arguments".to_string(),
                        ));
                    }
                    let arg_names = match &list[1] {
                        Value::List(args) => args
                            .iter()
                            .map(|v| match v {
                                Value::Symbol(s) => Ok(s.clone()),
                                _ => Err(RuntimeError::Message(format!(
                                    "lambda argument must be a symbol"
                                ))),
                            })
                            .collect::<Result<_, _>>()?,
                        _ => {
                            return Err(RuntimeError::Message(
                                "lambda second argument must be a list of symbols".to_string(),
                            ));
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

pub fn eval<'world>(
    expr: Value<'world>,
    env: EnvRef<'world>,
) -> Result<Value<'world>, RuntimeError> {
    trampoline(Thunk::Eval(expr, env))
}

pub fn parse_and_eval(program: &str) -> Option<Value<'_>> {
    let parsed: Vec<Value<'_>> = parse_lscheme(program).unwrap();
    let env = default_environment();
    let mut result = None;
    for val in parsed {
        result = Some(eval(val, Rc::clone(&env)).unwrap());
    }
    result
}

// mutates the environment and returns an expression to evaluate
fn eval_fn<'world>(
    func: &Value<'world>,
    args: &[Value<'world>],
) -> Result<Thunk<'world>, RuntimeError> {
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
            for (arg_name, evaluated_arg) in arg_names.iter().zip(args) {
                env.map.insert(arg_name.clone(), evaluated_arg.clone());
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
        let expected = Ok(vec![Value::String("hello".to_string())]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_symbol() {
        let program = "foo";
        let expected = Ok(vec![Value::Symbol("foo".to_string())]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_simple_list() {
        let program = "(+ 1 2)";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("+".to_string()),
            Value::Number(1.0),
            Value::Number(2.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_nested_list() {
        let program = "(define x (list 1 2))";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("define".to_string()),
            Value::Symbol("x".to_string()),
            Value::List(vec![
                Value::Symbol("list".to_string()),
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
                Value::Symbol("+".to_string()),
                Value::Number(1.0),
                Value::Number(2.0),
            ]),
            Value::List(vec![
                Value::Symbol("define".to_string()),
                Value::Symbol("a".to_string()),
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
            Value::Symbol("set!".to_string()),
            Value::Symbol("my-var".to_string()),
            Value::String("hello".to_string()),
            Value::Number(123.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let program = "  ( +   1   2   )  ";
        let expected = Ok(vec![Value::List(vec![
            Value::Symbol("+".to_string()),
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
        let result = parse_and_eval("(define my_add (lambda (a b) (+ a b)))").unwrap();
        assert!(result.is_null());

        let result =
            parse_and_eval("(define my_add (lambda (a b) (+ a b)))\n(my_add (+ 3 4) 5)").unwrap();
        assert_eq!(result, Value::Number(3.0 + 4.0 + 5.0));
    }
}
