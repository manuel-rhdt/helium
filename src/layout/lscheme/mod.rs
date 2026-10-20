use std::{cell::RefCell, collections::HashMap, fmt::Debug, rc::Rc};

use font_parse::TableRecord;
use pest::{Parser, error::{Error, ErrorVariant}, iterators::Pair};
use pest_derive::Parser;

use specs::{
    prelude::{Entities, Join, ReadStorage, SystemData}, shred::{self, Fetch}, Entity, Read
};

mod repl;
pub mod ecs_binding;
pub mod scheme_layout;

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

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Message(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for RuntimeError {}

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
            pub fn get_value(&self) -> Result<Value<'lt>, RuntimeError> {
                match self {
                    $(Self::$component_name(resource) => {
                        resource.get_value()
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
                    Value::Ecs(components) => components.get_value(),
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

impl From<Error<Rule>> for RuntimeError {
    fn from(err: Error<Rule>) -> Self {
        RuntimeError::Message(format!("parse error: {}", err))
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
    Bool(bool),
    Number(f64),
    String(String),
    Symbol(String),
    List(Vec<Value<'world>>),

    // The following values are created by the evaluator
    NativeFunc(for<'a> fn(&[Value<'a>]) -> Result<Value<'a>, RuntimeError>),

    // Closure-based native function (can capture 'world-lifetime data, e.g. ECS storages)
    NativeMethod(Rc<dyn Fn(&[Value<'world>]) -> Result<Value<'world>, RuntimeError> + 'world>),

    // Represents a function including its environment.
    Closure {
        env: EnvRef<'world>,
        arg_names: Vec<String>,
        body: Vec<Value<'world>>,
    },

    // First-class continuation (captured call stack)
    Continuation(Rc<CallStack<'world>>),

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

    /// R7RS truthiness: only #f is false.
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Bool(false))
    }
}

impl Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => write!(f, "()"),
            Self::Bool(true) => write!(f, "#t"),
            Self::Bool(false) => write!(f, "#f"),
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
            Self::NativeMethod(_) => f.debug_tuple("NativeMethod").finish(),
            Self::Continuation(_) => f.debug_tuple("Continuation").finish(),
            Self::Ecs(c) => write!(f, "{:?}", c),
        }
    }
}

impl PartialEq for Value<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(l0), Self::Bool(r0)) => l0 == r0,
            (Self::Number(l0), Self::Number(r0)) => l0 == r0,
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Symbol(l0), Self::Symbol(r0)) => l0 == r0,
            (Self::List(l0), Self::List(r0)) => l0 == r0,
            _ => false,
        }
    }
}

fn expect_number(val: &Value) -> Result<f64, RuntimeError> {
    match val {
        Value::Number(num) => Ok(*num),
        _ => Err(RuntimeError::Message(format!("expected number, got {:?}", val))),
    }
}

pub fn parse_lscheme<'world>(program: &str) -> Result<Vec<Value<'world>>, Error<Rule>> {
    fn custom_error(pair: &Pair<Rule>, message: String) -> Error<Rule> {
        Error::new_from_span(ErrorVariant::CustomError { message }, pair.as_span())
    }

    fn parse_value<'a>(pair: Pair<Rule>) -> Result<Value<'a>, Error<Rule>> {
        match pair.as_rule() {
            Rule::Bool => Ok(Value::Bool(pair.as_str() == "#t")),
            Rule::Number => {
                let n = pair.as_str().parse().map_err(|e| {
                    custom_error(&pair, format!("invalid number {:?}: {}", pair.as_str(), e))
                })?;
                Ok(Value::Number(n))
            }
            Rule::String => {
                let inner = pair
                    .clone()
                    .into_inner()
                    .next()
                    .ok_or_else(|| custom_error(&pair, "string without contents".to_string()))?;
                Ok(Value::String(inner.as_str().into()))
            }
            Rule::Ident => Ok(Value::Symbol(pair.as_str().into())),
            Rule::List => Ok(Value::List(
                pair.into_inner().map(parse_value).collect::<Result<_, _>>()?,
            )),
            Rule::Quote => {
                let inner = pair
                    .clone()
                    .into_inner()
                    .next()
                    .ok_or_else(|| custom_error(&pair, "quote without expression".to_string()))?;
                Ok(Value::List(vec![Value::Symbol("quote".into()), parse_value(inner)?]))
            }
            other => Err(custom_error(&pair, format!("unexpected rule: {:?}", other))),
        }
    }

    let program_pair = SchemeParser::parse(Rule::Program, program)?
        .next()
        .ok_or_else(|| {
            Error::new_from_pos(
                ErrorVariant::CustomError { message: "empty parse result".to_string() },
                pest::Position::from_start(program),
            )
        })?;

    program_pair.into_inner().map(parse_value).collect()
}

#[derive(Debug, Clone)]
pub struct Environment<'world> {
    map: HashMap<String, Value<'world>>,
    parent: Option<EnvRef<'world>>,
}

impl<'world> Environment<'world> {
    // Break Rc cycles by dropping all bindings. Call after eval to allow WriteStorage borrows to release.
    pub fn clear_bindings(&mut self) {
        self.map.clear();
    }
}

pub fn default_environment<'a>() -> Result<EnvRef<'a>, RuntimeError> {
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

    fn multiply<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        args.iter().try_fold(Value::Number(1.0), |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a * b))
        })
    }
    new_map.insert("*".to_string(), Value::NativeFunc(multiply));

    fn divide<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let first_arg = args
            .get(0)
            .ok_or(RuntimeError::Message("called / without arguments".to_string()))?
            .clone();
        (&args[1..]).iter().try_fold(first_arg, |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a / b))
        })
    }
    new_map.insert("/".to_string(), Value::NativeFunc(divide));

    fn num_eq<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("= requires at least 2 arguments".to_string()));
        }
        let first = expect_number(&args[0])?;
        let mut ok = true;
        for v in &args[1..] {
            ok &= expect_number(v)? == first;
        }
        Ok(Value::Bool(ok))
    }
    new_map.insert("=".to_string(), Value::NativeFunc(num_eq));

    fn num_lt<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("< requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev < n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert("<".to_string(), Value::NativeFunc(num_lt));

    fn num_gt<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("> requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev > n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert(">".to_string(), Value::NativeFunc(num_gt));

    fn num_lte<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("<= requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev <= n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert("<=".to_string(), Value::NativeFunc(num_lte));

    fn num_gte<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message(">= requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev >= n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert(">=".to_string(), Value::NativeFunc(num_gte));

    fn not<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("not requires exactly one argument".to_string()));
        }
        Ok(Value::Bool(!args[0].is_truthy()))
    }
    new_map.insert("not".to_string(), Value::NativeFunc(not));

    fn equal<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::Message("equal? requires exactly 2 arguments".to_string()));
        }
        Ok(Value::Bool(args[0] == args[1]))
    }
    new_map.insert("equal?".to_string(), Value::NativeFunc(equal));

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

    fn cons<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::Message("cons requires exactly 2 arguments".to_string()));
        }
        match &args[1] {
            Value::List(tail) => {
                let mut new_list = vec![args[0].clone()];
                new_list.extend(tail.iter().cloned());
                Ok(Value::List(new_list))
            }
            Value::Null => Ok(Value::List(vec![args[0].clone()])),
            _ => Err(RuntimeError::Message("cons: second argument must be a list".to_string())),
        }
    }
    new_map.insert("cons".to_string(), Value::NativeFunc(cons));

    fn car<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("car requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) if !list.is_empty() => Ok(list[0].clone()),
            _ => Err(RuntimeError::Message(format!("car: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("car".to_string(), Value::NativeFunc(car));

    fn cdr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("cdr requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) if !list.is_empty() => Ok(Value::List(list[1..].to_vec())),
            Value::List(_) => Err(RuntimeError::Message("cdr: empty list".to_string())),
            _ => Err(RuntimeError::Message(format!("cdr: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cdr".to_string(), Value::NativeFunc(cdr));

    fn list<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        Ok(Value::List(args.to_vec()))
    }
    new_map.insert("list".to_string(), Value::NativeFunc(list));

    fn is_null<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("null? requires exactly 1 argument".to_string()));
        }
        let result = match &args[0] {
            Value::Null => true,
            Value::List(v) => v.is_empty(),
            _ => false,
        };
        Ok(Value::Bool(result))
    }
    new_map.insert("null?".to_string(), Value::NativeFunc(is_null));

    fn is_pair<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("pair? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(matches!(&args[0], Value::List(v) if !v.is_empty())))
    }
    new_map.insert("pair?".to_string(), Value::NativeFunc(is_pair));

    fn is_number<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("number? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_number()))
    }
    new_map.insert("number?".to_string(), Value::NativeFunc(is_number));

    fn is_string_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_string()))
    }
    new_map.insert("string?".to_string(), Value::NativeFunc(is_string_pred));

    fn is_symbol<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("symbol? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_symbol()))
    }
    new_map.insert("symbol?".to_string(), Value::NativeFunc(is_symbol));

    fn is_boolean<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("boolean? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(matches!(args[0], Value::Bool(_))))
    }
    new_map.insert("boolean?".to_string(), Value::NativeFunc(is_boolean));

    fn display<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("display requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => print!("{}", s),
            other => print!("{:?}", other),
        }
        Ok(Value::Null)
    }
    new_map.insert("display".to_string(), Value::NativeFunc(display));

    fn newline<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        println!();
        Ok(Value::Null)
    }
    new_map.insert("newline".to_string(), Value::NativeFunc(newline));

    fn abs<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("abs requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.abs()))
    }
    new_map.insert("abs".to_string(), Value::NativeFunc(abs));

    fn floor_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("floor requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.floor()))
    }
    new_map.insert("floor".to_string(), Value::NativeFunc(floor_fn));

    fn ceiling_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("ceiling requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.ceil()))
    }
    new_map.insert("ceiling".to_string(), Value::NativeFunc(ceiling_fn));

    fn sqrt_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("sqrt requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.sqrt()))
    }
    new_map.insert("sqrt".to_string(), Value::NativeFunc(sqrt_fn));

    fn append<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let mut result = Vec::new();
        for arg in args {
            match arg {
                Value::List(items) => result.extend(items.iter().cloned()),
                Value::Null => {}
                _ => return Err(RuntimeError::Message(format!("append: expected list, got {:?}", arg))),
            }
        }
        Ok(Value::List(result))
    }
    new_map.insert("append".to_string(), Value::NativeFunc(append));

    fn reverse<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("reverse requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) => {
                let mut rev = list.clone();
                rev.reverse();
                Ok(Value::List(rev))
            }
            _ => Err(RuntimeError::Message("reverse: expected list".to_string())),
        }
    }
    new_map.insert("reverse".to_string(), Value::NativeFunc(reverse));

    fn number_to_string<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("number->string requires exactly 1 argument".to_string()));
        }
        let n = expect_number(&args[0])?;
        Ok(Value::String(if n == n.floor() { format!("{}", n as i64) } else { format!("{}", n) }))
    }
    new_map.insert("number->string".to_string(), Value::NativeFunc(number_to_string));

    fn string_to_number<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string->number requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => match s.parse::<f64>() {
                Ok(n) => Ok(Value::Number(n)),
                Err(_) => Ok(Value::Bool(false)),
            },
            _ => Err(RuntimeError::Message("string->number: expected string".to_string())),
        }
    }
    new_map.insert("string->number".to_string(), Value::NativeFunc(string_to_number));

    fn string_append<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let mut result = String::new();
        for arg in args {
            match arg {
                Value::String(s) => result.push_str(s),
                _ => return Err(RuntimeError::Message(format!("string-append: expected string, got {:?}", arg))),
            }
        }
        Ok(Value::String(result))
    }
    new_map.insert("string-append".to_string(), Value::NativeFunc(string_append));

    fn symbol_to_string<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("symbol->string requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::Symbol(s) => Ok(Value::String(s.clone())),
            _ => Err(RuntimeError::Message("symbol->string: expected symbol".to_string())),
        }
    }
    new_map.insert("symbol->string".to_string(), Value::NativeFunc(symbol_to_string));

    fn string_to_symbol<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string->symbol requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => Ok(Value::Symbol(s.clone())),
            _ => Err(RuntimeError::Message("string->symbol: expected string".to_string())),
        }
    }
    new_map.insert("string->symbol".to_string(), Value::NativeFunc(string_to_symbol));

    fn cadr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cadr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 2 => Ok(v[1].clone()),
            _ => Err(RuntimeError::Message(format!("cadr: expected list with >=2 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("cadr".to_string(), Value::NativeFunc(cadr));

    fn caddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("caddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 3 => Ok(v[2].clone()),
            _ => Err(RuntimeError::Message(format!("caddr: expected list with >=3 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("caddr".to_string(), Value::NativeFunc(caddr));

    fn cadddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cadddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 4 => Ok(v[3].clone()),
            _ => Err(RuntimeError::Message(format!("cadddr: expected list with >=4 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("cadddr".to_string(), Value::NativeFunc(cadddr));

    fn cddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 2 => Ok(Value::List(v[2..].to_vec())),
            Value::List(_) => Err(RuntimeError::Message("cddr: list too short".to_string())),
            _ => Err(RuntimeError::Message(format!("cddr: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cddr".to_string(), Value::NativeFunc(cddr));

    fn caar<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("caar requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if !v.is_empty() => match &v[0] {
                Value::List(inner) if !inner.is_empty() => Ok(inner[0].clone()),
                _ => Err(RuntimeError::Message("caar: car is not a non-empty list".to_string())),
            },
            _ => Err(RuntimeError::Message(format!("caar: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("caar".to_string(), Value::NativeFunc(caar));

    fn cdar<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cdar requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if !v.is_empty() => match &v[0] {
                Value::List(inner) if !inner.is_empty() => Ok(Value::List(inner[1..].to_vec())),
                Value::List(_) => Err(RuntimeError::Message("cdar: car is empty list".to_string())),
                _ => Err(RuntimeError::Message("cdar: car is not a list".to_string())),
            },
            _ => Err(RuntimeError::Message(format!("cdar: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cdar".to_string(), Value::NativeFunc(cdar));

    fn eq_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 { return Err(RuntimeError::Message("eq? requires 2 arguments".to_string())); }
        let result = match (&args[0], &args[1]) {
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::List(a), Value::List(b)) => a.is_empty() && b.is_empty(),
            (Value::Number(a), Value::Number(b)) => a == b,
            _ => false,
        };
        Ok(Value::Bool(result))
    }
    new_map.insert("eq?".to_string(), Value::NativeFunc(eq_pred));

    fn min_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.is_empty() { return Err(RuntimeError::Message("min requires at least 1 argument".to_string())); }
        let mut m = expect_number(&args[0])?;
        for v in &args[1..] { let n = expect_number(v)?; if n < m { m = n; } }
        Ok(Value::Number(m))
    }
    new_map.insert("min".to_string(), Value::NativeFunc(min_fn));

    fn max_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.is_empty() { return Err(RuntimeError::Message("max requires at least 1 argument".to_string())); }
        let mut m = expect_number(&args[0])?;
        for v in &args[1..] { let n = expect_number(v)?; if n > m { m = n; } }
        Ok(Value::Number(m))
    }
    new_map.insert("max".to_string(), Value::NativeFunc(max_fn));

    fn zero_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("zero? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? == 0.0))
    }
    new_map.insert("zero?".to_string(), Value::NativeFunc(zero_pred));

    fn positive_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("positive? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? > 0.0))
    }
    new_map.insert("positive?".to_string(), Value::NativeFunc(positive_pred));

    fn negative_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("negative? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? < 0.0))
    }
    new_map.insert("negative?".to_string(), Value::NativeFunc(negative_pred));

    fn list_ref<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 { return Err(RuntimeError::Message("list-ref requires 2 arguments".to_string())); }
        let idx = expect_number(&args[1])? as usize;
        match &args[0] {
            Value::List(v) => v.get(idx).cloned().ok_or_else(|| RuntimeError::Message(
                format!("list-ref: index {} out of bounds (length {})", idx, v.len())
            )),
            _ => Err(RuntimeError::Message(format!("list-ref: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("list-ref".to_string(), Value::NativeFunc(list_ref));

    let env = Rc::new(RefCell::new(Environment {
        map: new_map,
        parent: None,
    }));

    // Bootstrap higher-order functions defined in Scheme (requires working recursion)
    let bootstrap = r#"
(define (for-each f lst)
  (when (pair? lst)
    (f (car lst))
    (for-each f (cdr lst))))

(define (map f lst)
  (if (null? lst)
    '()
    (cons (f (car lst)) (map f (cdr lst)))))

(define (filter f lst)
  (cond
    ((null? lst) '())
    ((f (car lst)) (cons (car lst) (filter f (cdr lst))))
    (else (filter f (cdr lst)))))

(define (assoc key lst)
  (cond
    ((null? lst) #f)
    ((equal? (caar lst) key) (car lst))
    (else (assoc key (cdr lst)))))

(define (member x lst)
  (cond
    ((null? lst) #f)
    ((equal? x (car lst)) lst)
    (else (member x (cdr lst)))))
"#;
    for form in parse_lscheme(bootstrap)? {
        eval(form, Rc::clone(&env))?;
    }

    Ok(env)
}

type EnvRef<'world> = Rc<RefCell<Environment<'world>>>;

impl<'world> Environment<'world> {
    pub fn set(&mut self, k: String, v: Value<'world>) {
        self.map.insert(k, v);
    }

    /// Mutate an existing binding, walking up the parent chain.
    pub fn set_existing(&mut self, k: &str, v: Value<'world>) -> Result<(), RuntimeError> {
        if self.map.contains_key(k) {
            self.map.insert(k.to_string(), v);
            Ok(())
        } else if let Some(parent) = &self.parent {
            parent.borrow_mut().set_existing(k, v)
        } else {
            Err(RuntimeError::Message(format!("set!: undefined variable: {}", k)))
        }
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
    EvaluateSet {
        name: String,
        env: EnvRef<'world>,
    },
    If {
        consequent: Value<'world>,
        alternate: Option<Value<'world>>,
        env: EnvRef<'world>,
    },
    /// Evaluates a sequence, discarding intermediate values, tail-calling the last.
    Sequence {
        remaining: Vec<Value<'world>>,
        env: EnvRef<'world>,
    },
    And {
        remaining: Vec<Value<'world>>,
        env: EnvRef<'world>,
    },
    Or {
        remaining: Vec<Value<'world>>,
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

    pub fn pop_frame(&mut self) -> Result<Continuation<'world>, RuntimeError> {
        self.stack.pop().ok_or_else(|| {
            RuntimeError::Message("internal error: call stack underflow".to_string())
        })
    }
}

#[derive(Debug)]
enum Thunk<'world> {
    Value(Value<'world>),
    Return(Value<'world>),
    Eval(Value<'world>, EnvRef<'world>),
    TailCall(Value<'world>, EnvRef<'world>),
    InvokeContinuation(Rc<CallStack<'world>>, Value<'world>),
}

impl<'world> Continuation<'world> {
    fn apply_value(&mut self, val: Value<'world>) -> Result<Thunk<'world>, RuntimeError> {
        match self {
            Continuation::EvaluateDefine { name, env } => {
                env.borrow_mut().map.insert(name.clone(), val);
                Ok(Thunk::Return(Value::Null))
            }
            Continuation::EvaluateSet { name, env } => {
                env.borrow_mut().set_existing(name, val)?;
                Ok(Thunk::Return(Value::Null))
            }
            Continuation::If { consequent, alternate, env } => {
                if val.is_truthy() {
                    Ok(Thunk::TailCall(consequent.clone(), Rc::clone(env)))
                } else if let Some(alt) = alternate {
                    Ok(Thunk::TailCall(alt.clone(), Rc::clone(env)))
                } else {
                    Ok(Thunk::Return(Value::Null))
                }
            }
            Continuation::Sequence { remaining, env } => {
                if remaining.is_empty() {
                    Ok(Thunk::Return(val))
                } else if remaining.len() == 1 {
                    let last = remaining[0].clone();
                    Ok(Thunk::TailCall(last, Rc::clone(env)))
                } else {
                    let next = remaining.remove(0);
                    Ok(Thunk::Eval(next, Rc::clone(env)))
                }
            }
            Continuation::And { remaining, env } => {
                if !val.is_truthy() {
                    Ok(Thunk::Return(Value::Bool(false)))
                } else if remaining.is_empty() {
                    Ok(Thunk::Return(val))
                } else if remaining.len() == 1 {
                    let last = remaining[0].clone();
                    Ok(Thunk::TailCall(last, Rc::clone(env)))
                } else {
                    let next = remaining.remove(0);
                    Ok(Thunk::Eval(next, Rc::clone(env)))
                }
            }
            Continuation::Or { remaining, env } => {
                if val.is_truthy() {
                    Ok(Thunk::Return(val))
                } else if remaining.is_empty() {
                    Ok(Thunk::Return(Value::Bool(false)))
                } else if remaining.len() == 1 {
                    let last = remaining[0].clone();
                    Ok(Thunk::TailCall(last, Rc::clone(env)))
                } else {
                    let next = remaining.remove(0);
                    Ok(Thunk::Eval(next, Rc::clone(env)))
                }
            }
            Continuation::ApplyFunction {
                proc,
                args,
                evaluated_args,
                env,
                ..
            } => {
                if proc.is_none() {
                    *proc = Some(val);

                    if args.is_empty() {
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(&func, &[])?;
                        Ok(val)
                    } else {
                        let first_arg = args[0].clone();
                        Ok(Thunk::Eval(first_arg, Rc::clone(env)))
                    }
                } else {
                    if evaluated_args.len() >= args.len() {
                        return Err(RuntimeError::Message(
                            "internal error: continuation called after it was exhausted".to_string(),
                        ));
                    }
                    evaluated_args.push(val);
                    if evaluated_args.len() == args.len() {
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(func, evaluated_args)?;
                        Ok(val)
                    } else {
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
                stack.pop_frame()?;
                current_thunk = Thunk::Value(val);
            }
            Thunk::Eval(expr, env) => {
                current_thunk = eval_expr(expr, env, &mut stack)?;
            }
            Thunk::TailCall(expr, env) => {
                stack.pop_frame()?;
                current_thunk = Thunk::Eval(expr, env);
            }
            Thunk::InvokeContinuation(saved_stack, val) => {
                stack = (*saved_stack).clone();
                current_thunk = Thunk::Value(val);
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
                return Ok(Thunk::Value(Value::Null));
            }

            let head = &list[0];

            match head {
                Value::Symbol(x) if x == "define" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "define requires at least two arguments".to_string(),
                        ));
                    }
                    match &list[1] {
                        Value::Symbol(name) => {
                            let name = name.clone();
                            stack.push_frame(Continuation::EvaluateDefine {
                                name,
                                env: Rc::clone(&env),
                            });
                            Ok(Thunk::Eval(list[2].clone(), Rc::clone(&env)))
                        }
                        Value::List(func_spec) => {
                            // (define (f x y) body...) → (define f (lambda (x y) body...))
                            let fname = match &func_spec[0] {
                                Value::Symbol(s) => s.clone(),
                                _ => return Err(RuntimeError::Message(
                                    "define: function name must be a symbol".to_string(),
                                )),
                            };
                            let params = Value::List(func_spec[1..].to_vec());
                            let lambda = Value::List(
                                std::iter::once(Value::Symbol("lambda".into()))
                                    .chain(std::iter::once(params))
                                    .chain(list[2..].iter().cloned())
                                    .collect(),
                            );
                            let desugared = Value::List(vec![
                                Value::Symbol("define".into()),
                                Value::Symbol(fname),
                                lambda,
                            ]);
                            Ok(Thunk::Eval(desugared, env))
                        }
                        _ => Err(RuntimeError::Message(
                            "define: first argument must be a symbol or list".to_string(),
                        )),
                    }
                }
                Value::Symbol(x) if x == "set!" => {
                    if list.len() != 3 {
                        return Err(RuntimeError::Message(
                            "set! requires exactly two arguments".to_string(),
                        ));
                    }
                    let name = match &list[1] {
                        Value::Symbol(s) => s.clone(),
                        _ => return Err(RuntimeError::Message(
                            "set! first argument must be a symbol".to_string(),
                        )),
                    };
                    stack.push_frame(Continuation::EvaluateSet {
                        name,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(list[2].clone(), Rc::clone(&env)))
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
                        env: Rc::clone(&env),
                        arg_names,
                        body,
                    };
                    Ok(Thunk::Value(closure))
                }
                Value::Symbol(x) if x == "if" => {
                    if list.len() < 3 || list.len() > 4 {
                        return Err(RuntimeError::Message(
                            "if requires 2 or 3 arguments".to_string(),
                        ));
                    }
                    let consequent = list[2].clone();
                    let alternate = list.get(3).cloned();
                    stack.push_frame(Continuation::If {
                        consequent,
                        alternate,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(list[1].clone(), env))
                }
                Value::Symbol(x) if x == "begin" => {
                    let exprs = list[1..].to_vec();
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Null));
                    }
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(exprs[0].clone(), env));
                    }
                    let first = exprs[0].clone();
                    let rest = exprs[1..].to_vec();
                    stack.push_frame(Continuation::Sequence {
                        remaining: rest,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(first, env))
                }
                Value::Symbol(x) if x == "quote" => {
                    if list.len() != 2 {
                        return Err(RuntimeError::Message(
                            "quote requires exactly one argument".to_string(),
                        ));
                    }
                    Ok(Thunk::Value(list[1].clone()))
                }
                Value::Symbol(x) if x == "let" => {
                    // (let ((x1 v1) ...) body...) → ((lambda (x1 ...) body...) v1 ...)
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "let requires bindings and body".to_string(),
                        ));
                    }
                    let bindings = match &list[1] {
                        Value::List(b) => b.clone(),
                        _ => return Err(RuntimeError::Message(
                            "let: bindings must be a list".to_string(),
                        )),
                    };
                    let mut params = Vec::new();
                    let mut vals = Vec::new();
                    for binding in &bindings {
                        match binding {
                            Value::List(pair) if pair.len() == 2 => {
                                params.push(pair[0].clone());
                                vals.push(pair[1].clone());
                            }
                            _ => return Err(RuntimeError::Message(
                                "let: each binding must be a (name value) pair".to_string(),
                            )),
                        }
                    }
                    let lambda = Value::List(
                        std::iter::once(Value::Symbol("lambda".into()))
                            .chain(std::iter::once(Value::List(params)))
                            .chain(list[2..].iter().cloned())
                            .collect(),
                    );
                    let application = Value::List(
                        std::iter::once(lambda).chain(vals).collect(),
                    );
                    Ok(Thunk::Eval(application, env))
                }
                Value::Symbol(x) if x == "let*" => {
                    // (let* () body...) → (begin body...)
                    // (let* ((x v) rest...) body...) → (let ((x v)) (let* (rest...) body...))
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "let*: requires bindings and body".to_string(),
                        ));
                    }
                    let bindings = match &list[1] {
                        Value::List(b) => b.clone(),
                        _ => return Err(RuntimeError::Message(
                            "let*: bindings must be a list".to_string(),
                        )),
                    };
                    if bindings.is_empty() {
                        let begin_expr = Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list[2..].iter().cloned())
                                .collect(),
                        );
                        Ok(Thunk::Eval(begin_expr, env))
                    } else {
                        let first_binding = Value::List(vec![bindings[0].clone()]);
                        let rest_bindings = Value::List(bindings[1..].to_vec());
                        let inner = Value::List(
                            std::iter::once(Value::Symbol("let*".into()))
                                .chain(std::iter::once(rest_bindings))
                                .chain(list[2..].iter().cloned())
                                .collect(),
                        );
                        let outer = Value::List(vec![
                            Value::Symbol("let".into()),
                            first_binding,
                            inner,
                        ]);
                        Ok(Thunk::Eval(outer, env))
                    }
                }
                Value::Symbol(x) if x == "and" => {
                    let exprs = list[1..].to_vec();
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Bool(true)));
                    }
                    let first = exprs[0].clone();
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(first, env));
                    }
                    let rest = exprs[1..].to_vec();
                    stack.push_frame(Continuation::And {
                        remaining: rest,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(first, env))
                }
                Value::Symbol(x) if x == "or" => {
                    let exprs = list[1..].to_vec();
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Bool(false)));
                    }
                    let first = exprs[0].clone();
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(first, env));
                    }
                    let rest = exprs[1..].to_vec();
                    stack.push_frame(Continuation::Or {
                        remaining: rest,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(first, env))
                }
                Value::Symbol(x) if x == "cond" => {
                    fn desugar_cond<'w>(clauses: &[Value<'w>]) -> Result<Value<'w>, RuntimeError> {
                        if clauses.is_empty() {
                            return Ok(Value::Null);
                        }
                        let clause = match &clauses[0] {
                            Value::List(l) if !l.is_empty() => l,
                            _ => return Err(RuntimeError::Message(
                                "cond: each clause must be a non-empty list".to_string(),
                            )),
                        };
                        let test = &clause[0];
                        let body = if clause.len() == 2 {
                            clause[1].clone()
                        } else {
                            Value::List(
                                std::iter::once(Value::Symbol("begin".into()))
                                    .chain(clause[1..].iter().cloned())
                                    .collect(),
                            )
                        };
                        if matches!(test, Value::Symbol(s) if s == "else") {
                            Ok(body)
                        } else {
                            let rest = desugar_cond(&clauses[1..])?;
                            Ok(Value::List(vec![
                                Value::Symbol("if".into()),
                                test.clone(),
                                body,
                                rest,
                            ]))
                        }
                    }
                    let desugared = desugar_cond(&list[1..])?;
                    Ok(Thunk::Eval(desugared, env))
                }
                Value::Symbol(x) if x == "when" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message("when requires test and body".to_string()));
                    }
                    let test = list[1].clone();
                    let body = if list.len() == 3 {
                        list[2].clone()
                    } else {
                        Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list[2..].iter().cloned())
                                .collect(),
                        )
                    };
                    let if_expr = Value::List(vec![Value::Symbol("if".into()), test, body]);
                    Ok(Thunk::Eval(if_expr, env))
                }
                Value::Symbol(x) if x == "unless" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message("unless requires test and body".to_string()));
                    }
                    let test = list[1].clone();
                    let body = if list.len() == 3 {
                        list[2].clone()
                    } else {
                        Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list[2..].iter().cloned())
                                .collect(),
                        )
                    };
                    let not_test = Value::List(vec![Value::Symbol("not".into()), test]);
                    let if_expr = Value::List(vec![Value::Symbol("if".into()), not_test, body]);
                    Ok(Thunk::Eval(if_expr, env))
                }
                Value::Symbol(x) if x == "call/cc" || x == "call-with-current-continuation" => {
                    if list.len() != 2 {
                        return Err(RuntimeError::Message(
                            "call/cc requires exactly one argument".to_string(),
                        ));
                    }
                    let captured_k = Value::Continuation(Rc::new(stack.clone()));
                    stack.push_frame(Continuation::ApplyFunction {
                        proc: None,
                        args: vec![captured_k],
                        evaluated_args: vec![],
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(list[1].clone(), env))
                }
                head => {
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
        // Self-evaluating forms
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
    let env = default_environment().unwrap();
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
    match func {
        Value::NativeFunc(f) => Ok(Thunk::Return(f(args)?)),
        Value::NativeMethod(f) => Ok(Thunk::Return(f(args)?)),
        Value::Closure {
            env: lambda_env,
            arg_names,
            body,
        } => {
            let env_ref = Rc::new(RefCell::new(Environment {
                map: HashMap::new(),
                parent: Some(Rc::clone(&lambda_env)),
            }));
            for (arg_name, evaluated_arg) in arg_names.iter().zip(args) {
                env_ref.borrow_mut().map.insert(arg_name.clone(), evaluated_arg.clone());
            }
            if body.len() == 1 {
                Ok(Thunk::TailCall(body[0].clone(), env_ref))
            } else {
                // Wrap multi-expr body in (begin ...) for proper sequencing
                let begin_expr = Value::List(
                    std::iter::once(Value::Symbol("begin".into()))
                        .chain(body.iter().cloned())
                        .collect(),
                );
                Ok(Thunk::TailCall(begin_expr, env_ref))
            }
        }
        Value::Continuation(saved_stack) => {
            let arg = match args {
                [] => Value::Null,
                [arg] => arg.clone(),
                _ => {
                    return Err(RuntimeError::Message(format!(
                        "continuation takes at most 1 argument, got {}",
                        args.len()
                    )))
                }
            };
            Ok(Thunk::InvokeContinuation(Rc::clone(saved_stack), arg))
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

    #[test]
    fn test_booleans() {
        assert_eq!(parse_lscheme("#t").unwrap(), vec![Value::Bool(true)]);
        assert_eq!(parse_lscheme("#f").unwrap(), vec![Value::Bool(false)]);
        assert_eq!(parse_and_eval("#t").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("#f").unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_comments() {
        let result = parse_and_eval("; this is a comment\n42").unwrap();
        assert_eq!(result, Value::Number(42.0));
        let result = parse_and_eval("(+ 1 2) ; inline comment").unwrap();
        assert_eq!(result, Value::Number(3.0));
    }

    #[test]
    fn test_quote() {
        assert_eq!(
            parse_and_eval("'foo").unwrap(),
            Value::Symbol("foo".to_string())
        );
        assert_eq!(
            parse_and_eval("(quote foo)").unwrap(),
            Value::Symbol("foo".to_string())
        );
        assert_eq!(
            parse_and_eval("'(1 2 3)").unwrap(),
            Value::List(vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)])
        );
    }

    #[test]
    fn test_if() {
        assert_eq!(parse_and_eval("(if #t 1 2)").unwrap(), Value::Number(1.0));
        assert_eq!(parse_and_eval("(if #f 1 2)").unwrap(), Value::Number(2.0));
        assert!(parse_and_eval("(if #f 1)").unwrap().is_null());
        // Only #f is falsy — 0 is truthy
        assert_eq!(parse_and_eval("(if 0 1 2)").unwrap(), Value::Number(1.0));
    }

    #[test]
    fn test_begin() {
        assert_eq!(parse_and_eval("(begin 1 2 3)").unwrap(), Value::Number(3.0));
        assert_eq!(parse_and_eval("(begin 42)").unwrap(), Value::Number(42.0));
    }

    #[test]
    fn test_set() {
        let result = parse_and_eval("(define x 1) (set! x 42) x").unwrap();
        assert_eq!(result, Value::Number(42.0));
    }

    #[test]
    fn test_let() {
        let result = parse_and_eval("(let ((x 1) (y 2)) (+ x y))").unwrap();
        assert_eq!(result, Value::Number(3.0));
    }

    #[test]
    fn test_let_star() {
        let result = parse_and_eval("(let* ((x 1) (y (+ x 1))) y)").unwrap();
        assert_eq!(result, Value::Number(2.0));
    }

    #[test]
    fn test_define_shorthand() {
        let result = parse_and_eval("(define (double x) (* x 2)) (double 5)").unwrap();
        assert_eq!(result, Value::Number(10.0));
    }

    #[test]
    fn test_multi_expr_lambda_body() {
        let result = parse_and_eval("((lambda (x) (define y (* x 2)) y) 5)").unwrap();
        assert_eq!(result, Value::Number(10.0));
    }

    #[test]
    fn test_and_or() {
        assert_eq!(parse_and_eval("(and #t #t)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(and #t #f)").unwrap(), Value::Bool(false));
        assert_eq!(parse_and_eval("(and)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(or #f #t)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(or #f #f)").unwrap(), Value::Bool(false));
        assert_eq!(parse_and_eval("(or)").unwrap(), Value::Bool(false));
        // and returns last truthy value
        assert_eq!(parse_and_eval("(and 1 2)").unwrap(), Value::Number(2.0));
        // or returns first truthy value
        assert_eq!(parse_and_eval("(or #f 42)").unwrap(), Value::Number(42.0));
    }

    #[test]
    fn test_cond() {
        let result = parse_and_eval("(cond (#f 1) (#t 2) (else 3))").unwrap();
        assert_eq!(result, Value::Number(2.0));
        let result = parse_and_eval("(cond (#f 1) (#f 2) (else 3))").unwrap();
        assert_eq!(result, Value::Number(3.0));
    }

    #[test]
    fn test_call_cc_early_exit() {
        let result = parse_and_eval("(call/cc (lambda (k) (k 42) 99))").unwrap();
        assert_eq!(result, Value::Number(42.0));
    }

    #[test]
    fn test_call_cc_no_escape() {
        let result = parse_and_eval("(call/cc (lambda (k) (+ 1 2)))").unwrap();
        assert_eq!(result, Value::Number(3.0));
    }

    #[test]
    fn test_list_builtins() {
        assert_eq!(parse_and_eval("(car '(1 2 3))").unwrap(), Value::Number(1.0));
        assert_eq!(
            parse_and_eval("(cdr '(1 2 3))").unwrap(),
            Value::List(vec![Value::Number(2.0), Value::Number(3.0)])
        );
        assert_eq!(
            parse_and_eval("(cons 1 '(2 3))").unwrap(),
            Value::List(vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)])
        );
        assert_eq!(parse_and_eval("(null? '())").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(null? '(1))").unwrap(), Value::Bool(false));
        assert_eq!(parse_and_eval("(pair? '(1))").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(pair? '())").unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_not() {
        assert_eq!(parse_and_eval("(not #f)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(not #t)").unwrap(), Value::Bool(false));
        assert_eq!(parse_and_eval("(not 0)").unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_equal() {
        assert_eq!(parse_and_eval("(equal? 1 1)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(equal? 1 2)").unwrap(), Value::Bool(false));
        assert_eq!(
            parse_and_eval("(equal? '(1 2) '(1 2))").unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_arithmetic() {
        assert_eq!(parse_and_eval("(* 3 4)").unwrap(), Value::Number(12.0));
        assert_eq!(parse_and_eval("(/ 10 2)").unwrap(), Value::Number(5.0));
        assert_eq!(parse_and_eval("(= 3 3)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(< 1 2)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(> 2 1)").unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_comparison_or_equal() {
        assert_eq!(parse_and_eval("(>= 2 2)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(>= 1 2)").unwrap(), Value::Bool(false));
        assert_eq!(parse_and_eval("(<= 1 2)").unwrap(), Value::Bool(true));
        assert_eq!(parse_and_eval("(<= 3 2)").unwrap(), Value::Bool(false));
    }

    fn eval_err(program: &str) -> RuntimeError {
        let env = default_environment().unwrap();
        let mut last = Ok(Value::Null);
        for form in parse_lscheme(program).unwrap() {
            last = eval(form, Rc::clone(&env));
            if last.is_err() {
                break;
            }
        }
        last.expect_err("expected the program to fail")
    }

    #[test]
    fn test_parse_error_is_returned() {
        assert!(parse_lscheme("(+ 1 2").is_err());
    }

    #[test]
    fn test_errors_are_raised() {
        // type error in `=` must not be treated as "not equal"
        assert!(matches!(eval_err("(= 1 'a)"), RuntimeError::Message(_)));
        assert!(matches!(eval_err("(undefined-fn 1)"), RuntimeError::Message(_)));
        assert!(matches!(eval_err("(car '())"), RuntimeError::Message(_)));
        assert!(matches!(eval_err("(call/cc (lambda (k) (k 1 2)))"), RuntimeError::Message(_)));
    }

    #[test]
    fn test_error_in_callback_propagates() {
        // errors raised inside nested evaluation reach the caller
        assert!(matches!(eval_err("((lambda () (+ 1 'a)))"), RuntimeError::Message(_)));
    }
}
