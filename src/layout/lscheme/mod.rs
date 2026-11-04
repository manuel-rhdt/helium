use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::{Hash, Hasher},
    rc::Rc,
};

use pest::{
    Parser,
    error::{Error, ErrorVariant},
    iterators::Pair,
};
use pest_derive::Parser;
use smallvec::SmallVec;

use specs::{
    Entity, Read,
    prelude::{Entities, Join, ReadStorage, SystemData},
    shred::{self},
};

mod default_env;
pub mod ecs_binding;
mod repl;
pub mod scheme_layout;

use crate::style::properties::computed::ComputedValues;

pub use self::default_env::default_environment;
pub use self::repl::repl;

use super::components::RootNode;

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
        Ok(Value::list(value))
    }
}

impl<'world, T: specs::Component> GetValue<'world> for ReadStorage<'world, T>
where
    for<'a> Value<'world>: From<&'a T>,
{
    fn get_value(&self) -> Result<Value<'world>, RuntimeError> {
        let value = self.join().map(Into::into).collect::<Vec<_>>();
        Ok(Value::list(value))
    }
}

impl From<Error<Rule>> for RuntimeError {
    fn from(err: Error<Rule>) -> Self {
        RuntimeError::Message(format!("parse error: {}", err))
    }
}

impl From<Entity> for Value<'_> {
    fn from(ent: Entity) -> Self {
        Value::Entity(ent)
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

/// A persistent list: cloning, `cdr` and `cons` share structure instead of copying the elements.
pub type List<'world> = imbl::GenericVector<Value<'world>, imbl::shared_ptr::RcK>;

#[derive(Clone)]
pub enum Value<'world> {
    // immutable values
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Symbol(Symbol),
    List(List<'world>),
    Entity(Entity),

    // The following values are created by the evaluator
    NativeFunc(for<'a> fn(&[Value<'a>]) -> Result<Value<'a>, RuntimeError>),

    // Closure-based native function (can capture 'world-lifetime data, e.g. ECS storages)
    NativeMethod(Rc<dyn Fn(&[Value<'world>]) -> Result<Value<'world>, RuntimeError> + 'world>),

    // Represents a function including its environment.
    Closure {
        globals: EnvRef<'world>,
        function: Rc<FunctionProto<'world>>,
        upvalues: Rc<[Upvalue<'world>]>,
    },

    // First-class continuation (captured call stack)
    Continuation(Rc<CallStack<'world>>),

    // Native types
    Ecs(Rc<Resource<'world>>),
}

type Upvalue<'world> = Rc<RefCell<Option<Value<'world>>>>;

#[derive(Debug, Clone)]
enum Expr<'world> {
    Literal(Value<'world>),
    Global(Symbol),
    Local(usize),
    Upvalue(usize),
    Define(BindingRef, Box<Expr<'world>>),
    Set(BindingRef, Box<Expr<'world>>),
    If(
        Box<Expr<'world>>,
        Box<Expr<'world>>,
        Option<Box<Expr<'world>>>,
    ),
    Begin(Vec<Expr<'world>>),
    And(Vec<Expr<'world>>),
    Or(Vec<Expr<'world>>),
    Lambda(Rc<FunctionProto<'world>>),
    Call(Box<Expr<'world>>, Vec<Expr<'world>>),
    CallCc(Box<Expr<'world>>),
}

#[derive(Debug, Clone)]
enum BindingRef {
    Global(Symbol),
    Local(usize),
    Upvalue(usize),
}

#[derive(Debug, Clone)]
enum CaptureSource {
    Local(usize),
    Upvalue(usize),
}

#[derive(Debug, Clone)]
struct FunctionProto<'world> {
    parameter_slots: Vec<usize>,
    local_count: usize,
    captures: Vec<CaptureSource>,
    body: Rc<Expr<'world>>,
}

#[derive(Debug, Clone)]
enum Slot<'world> {
    Uninitialized,
    Value(Value<'world>),
    Shared(Upvalue<'world>),
}

#[derive(Debug, Clone)]
struct ActivationFrame<'world> {
    slots: Vec<Slot<'world>>,
}

impl<'world> Slot<'world> {
    fn read(&self) -> Option<Value<'world>> {
        match self {
            Self::Uninitialized => None,
            Self::Value(value) => Some(value.clone()),
            Self::Shared(value) => value.borrow().clone(),
        }
    }

    fn write(&mut self, value: Value<'world>) {
        match self {
            Self::Shared(shared) => *shared.borrow_mut() = Some(value),
            Self::Uninitialized | Self::Value(_) => *self = Self::Value(value),
        }
    }

    fn promote(&mut self) -> Upvalue<'world> {
        match self {
            Self::Shared(shared) => Rc::clone(shared),
            Self::Uninitialized => {
                let shared = Rc::new(RefCell::new(None));
                *self = Self::Shared(Rc::clone(&shared));
                shared
            }
            Self::Value(value) => {
                let shared = Rc::new(RefCell::new(Some(value.clone())));
                *self = Self::Shared(Rc::clone(&shared));
                shared
            }
        }
    }
}

struct FunctionCompiler {
    locals: Vec<Symbol>,
    local_slots: HashMap<Symbol, usize>,
    captures: Vec<(Symbol, CaptureSource)>,
    capture_slots: HashMap<Symbol, usize>,
    is_function: bool,
}

impl FunctionCompiler {
    fn top_level() -> Self {
        Self {
            locals: Vec::new(),
            local_slots: HashMap::new(),
            captures: Vec::new(),
            capture_slots: HashMap::new(),
            is_function: false,
        }
    }

    fn add_local(&mut self, name: Symbol) -> usize {
        if let Some(&slot) = self.local_slots.get(&name) {
            return slot;
        }
        let slot = self.locals.len();
        self.locals.push(name.clone());
        self.local_slots.insert(name, slot);
        slot
    }

    fn resolve(&self, name: &Symbol) -> BindingRef {
        if let Some(&slot) = self.local_slots.get(name) {
            BindingRef::Local(slot)
        } else if let Some(&slot) = self.capture_slots.get(name) {
            BindingRef::Upvalue(slot)
        } else {
            BindingRef::Global(name.clone())
        }
    }

    fn visible_bindings(&self) -> Vec<(Symbol, CaptureSource)> {
        let mut visible = Vec::new();
        for (slot, name) in self.locals.iter().enumerate() {
            visible.push((name.clone(), CaptureSource::Local(slot)));
        }
        for (slot, (name, _)) in self.captures.iter().enumerate() {
            if !self.local_slots.contains_key(name) {
                visible.push((name.clone(), CaptureSource::Upvalue(slot)));
            }
        }
        visible
    }
}

fn compile_function<'world>(
    parameters: &[Symbol],
    body: &[Value<'world>],
    outer: Vec<(Symbol, CaptureSource)>,
) -> Result<FunctionProto<'world>, RuntimeError> {
    let mut compiler = FunctionCompiler::top_level();
    compiler.is_function = true;

    let parameter_slots = parameters
        .iter()
        .map(|name| compiler.add_local(name.clone()))
        .collect();
    for expr in body {
        collect_definitions(expr, &mut compiler);
    }

    for (name, source) in outer {
        if compiler.local_slots.contains_key(&name) || compiler.capture_slots.contains_key(&name) {
            continue;
        }
        let slot = compiler.captures.len();
        compiler.capture_slots.insert(name.clone(), slot);
        compiler.captures.push((name, source));
    }

    let body = compile_forms(body, &mut compiler)?;
    Ok(FunctionProto {
        parameter_slots,
        local_count: compiler.locals.len(),
        captures: compiler
            .captures
            .into_iter()
            .map(|(_, source)| source)
            .collect(),
        body: Rc::new(body),
    })
}

fn collect_definitions(expr: &Value<'_>, compiler: &mut FunctionCompiler) {
    let Value::List(list) = expr else {
        return;
    };
    if list.is_empty() {
        return;
    }

    match &list[0] {
        Value::Symbol(form) if form == "quote" || form == "lambda" => return,
        Value::Symbol(form) if form == "define" => {
            if let Some(target) = list.get(1) {
                match target {
                    Value::Symbol(name) => {
                        compiler.add_local(name.clone());
                        if let Some(value) = list.get(2) {
                            collect_definitions(value, compiler);
                        }
                    }
                    Value::List(spec) if !spec.is_empty() => {
                        if let Value::Symbol(name) = &spec[0] {
                            compiler.add_local(name.clone());
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        Value::Symbol(form) if form == "let" || form == "let*" => {
            if let Some(Value::List(bindings)) = list.get(1) {
                for binding in bindings {
                    if let Value::List(pair) = binding {
                        if let Some(value) = pair.get(1) {
                            collect_definitions(value, compiler);
                        }
                    }
                }
            }
            return;
        }
        _ => {}
    }

    for child in list.iter().skip(1) {
        collect_definitions(child, compiler);
    }
}

fn compile_forms<'world>(
    forms: &[Value<'world>],
    compiler: &mut FunctionCompiler,
) -> Result<Expr<'world>, RuntimeError> {
    match forms {
        [] => Ok(Expr::Literal(Value::Null)),
        [form] => compile_expr(form, compiler),
        _ => forms
            .iter()
            .map(|form| compile_expr(form, compiler))
            .collect::<Result<Vec<_>, _>>()
            .map(Expr::Begin),
    }
}

fn compile_lambda<'world>(
    parameters: &[Symbol],
    body: &[Value<'world>],
    compiler: &FunctionCompiler,
) -> Result<Expr<'world>, RuntimeError> {
    Ok(Expr::Lambda(Rc::new(compile_function(
        parameters,
        body,
        compiler.visible_bindings(),
    )?)))
}

fn compile_expr<'world>(
    expr: &Value<'world>,
    compiler: &mut FunctionCompiler,
) -> Result<Expr<'world>, RuntimeError> {
    match expr {
        Value::Symbol(name) => Ok(match compiler.resolve(name) {
            BindingRef::Global(name) => Expr::Global(name),
            BindingRef::Local(slot) => Expr::Local(slot),
            BindingRef::Upvalue(slot) => Expr::Upvalue(slot),
        }),
        Value::List(list) if list.is_empty() => Ok(Expr::Literal(Value::Null)),
        Value::List(list) => {
            let head = &list[0];
            match head {
                Value::Symbol(form) if form == "quote" => {
                    if list.len() != 2 {
                        return Err(RuntimeError::Message(
                            "quote requires exactly one argument".to_string(),
                        ));
                    }
                    Ok(Expr::Literal(list[1].clone()))
                }
                Value::Symbol(form) if form == "define" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "define requires at least two arguments".to_string(),
                        ));
                    }
                    let (name, value) = match &list[1] {
                        Value::Symbol(name) => (name.clone(), compile_expr(&list[2], compiler)?),
                        Value::List(spec) if !spec.is_empty() => {
                            let name = match &spec[0] {
                                Value::Symbol(name) => name.clone(),
                                _ => {
                                    return Err(RuntimeError::Message(
                                        "define: function name must be a symbol".to_string(),
                                    ));
                                }
                            };
                            let parameters = spec
                                .iter()
                                .skip(1)
                                .map(|arg| match arg {
                                    Value::Symbol(name) => Ok(name.clone()),
                                    _ => Err(RuntimeError::Message(
                                        "lambda argument must be a symbol".to_string(),
                                    )),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            let value = compile_lambda(
                                &parameters,
                                &list.iter().skip(2).cloned().collect::<Vec<_>>(),
                                compiler,
                            )?;
                            (name, value)
                        }
                        _ => {
                            return Err(RuntimeError::Message(
                                "define: first argument must be a symbol or list".to_string(),
                            ));
                        }
                    };
                    let target = if compiler.is_function {
                        BindingRef::Local(*compiler.local_slots.get(&name).unwrap())
                    } else {
                        BindingRef::Global(name)
                    };
                    Ok(Expr::Define(target, Box::new(value)))
                }
                Value::Symbol(form) if form == "set!" => {
                    if list.len() != 3 {
                        return Err(RuntimeError::Message(
                            "set! requires exactly two arguments".to_string(),
                        ));
                    }
                    let name = match &list[1] {
                        Value::Symbol(name) => name,
                        _ => {
                            return Err(RuntimeError::Message(
                                "set! first argument must be a symbol".to_string(),
                            ));
                        }
                    };
                    Ok(Expr::Set(
                        compiler.resolve(name),
                        Box::new(compile_expr(&list[2], compiler)?),
                    ))
                }
                Value::Symbol(form) if form == "lambda" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "lambda requires at least two arguments".to_string(),
                        ));
                    }
                    let parameters = match &list[1] {
                        Value::List(args) => args
                            .iter()
                            .map(|arg| match arg {
                                Value::Symbol(name) => Ok(name.clone()),
                                _ => Err(RuntimeError::Message(
                                    "lambda argument must be a symbol".to_string(),
                                )),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => {
                            return Err(RuntimeError::Message(
                                "lambda second argument must be a list of symbols".to_string(),
                            ));
                        }
                    };
                    compile_lambda(
                        &parameters,
                        &list.iter().skip(2).cloned().collect::<Vec<_>>(),
                        compiler,
                    )
                }
                Value::Symbol(form) if form == "if" => {
                    if list.len() < 3 || list.len() > 4 {
                        return Err(RuntimeError::Message(
                            "if requires 2 or 3 arguments".to_string(),
                        ));
                    }
                    Ok(Expr::If(
                        Box::new(compile_expr(&list[1], compiler)?),
                        Box::new(compile_expr(&list[2], compiler)?),
                        list.get(3)
                            .map(|alternate| compile_expr(alternate, compiler).map(Box::new))
                            .transpose()?,
                    ))
                }
                Value::Symbol(form) if form == "begin" => {
                    compile_forms(&list.iter().skip(1).cloned().collect::<Vec<_>>(), compiler)
                }
                Value::Symbol(form) if form == "let" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "let requires bindings and body".to_string(),
                        ));
                    }
                    let bindings = match &list[1] {
                        Value::List(bindings) => bindings,
                        _ => {
                            return Err(RuntimeError::Message(
                                "let: bindings must be a list".to_string(),
                            ));
                        }
                    };
                    let mut parameters = Vec::new();
                    let mut arguments = Vec::new();
                    for binding in bindings {
                        match binding {
                            Value::List(pair) if pair.len() == 2 => match &pair[0] {
                                Value::Symbol(name) => {
                                    parameters.push(name.clone());
                                    arguments.push(compile_expr(&pair[1], compiler)?);
                                }
                                _ => {
                                    return Err(RuntimeError::Message(
                                        "let: binding name must be a symbol".to_string(),
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::Message(
                                    "let: each binding must be a (name value) pair".to_string(),
                                ));
                            }
                        }
                    }
                    let function = compile_lambda(
                        &parameters,
                        &list.iter().skip(2).cloned().collect::<Vec<_>>(),
                        compiler,
                    )?;
                    Ok(Expr::Call(Box::new(function), arguments))
                }
                Value::Symbol(form) if form == "let*" => {
                    let expanded = expand_let_star(list)?;
                    compile_expr(&expanded, compiler)
                }
                Value::Symbol(form) if form == "and" || form == "or" => {
                    let values = list
                        .iter()
                        .skip(1)
                        .map(|value| compile_expr(value, compiler))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(if form == "and" {
                        Expr::And(values)
                    } else {
                        Expr::Or(values)
                    })
                }
                Value::Symbol(form) if form == "cond" => {
                    let expanded = expand_cond(&list.skip(1))?;
                    compile_expr(&expanded, compiler)
                }
                Value::Symbol(form) if form == "when" || form == "unless" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(format!(
                            "{} requires a test and body",
                            form
                        )));
                    }
                    let test = compile_expr(&list[1], compiler)?;
                    let body =
                        compile_forms(&list.iter().skip(2).cloned().collect::<Vec<_>>(), compiler)?;
                    let (test, consequent, alternate) = if form == "when" {
                        (test, body, None)
                    } else {
                        (test, Expr::Literal(Value::Null), Some(Box::new(body)))
                    };
                    Ok(Expr::If(Box::new(test), Box::new(consequent), alternate))
                }
                Value::Symbol(form)
                    if form == "call/cc" || form == "call-with-current-continuation" =>
                {
                    if list.len() != 2 {
                        return Err(RuntimeError::Message(
                            "call/cc requires exactly one argument".to_string(),
                        ));
                    }
                    Ok(Expr::CallCc(Box::new(compile_expr(&list[1], compiler)?)))
                }
                _ => {
                    let function = compile_expr(head, compiler)?;
                    let arguments = list
                        .iter()
                        .skip(1)
                        .map(|argument| compile_expr(argument, compiler))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(Expr::Call(Box::new(function), arguments))
                }
            }
        }
        literal => Ok(Expr::Literal(literal.clone())),
    }
}

fn expand_let_star<'world>(list: &List<'world>) -> Result<Value<'world>, RuntimeError> {
    if list.len() < 3 {
        return Err(RuntimeError::Message(
            "let*: requires bindings and body".to_string(),
        ));
    }
    let bindings = match &list[1] {
        Value::List(bindings) => bindings,
        _ => {
            return Err(RuntimeError::Message(
                "let*: bindings must be a list".to_string(),
            ));
        }
    };
    fn nest<'world>(bindings: &List<'world>, body: &[Value<'world>]) -> Value<'world> {
        if bindings.is_empty() {
            return Value::List(
                std::iter::once(Value::Symbol("begin".into()))
                    .chain(body.iter().cloned())
                    .collect(),
            );
        }

        let inner = nest(&bindings.skip(1), body);
        Value::list(vec![
            Value::Symbol("let".into()),
            Value::list(vec![bindings[0].clone()]),
            inner,
        ])
    }

    let body: Vec<_> = list.iter().skip(2).cloned().collect();
    Ok(nest(bindings, &body))
}

fn expand_cond<'world>(clauses: &List<'world>) -> Result<Value<'world>, RuntimeError> {
    if clauses.is_empty() {
        return Ok(Value::Null);
    }
    let clause = match &clauses[0] {
        Value::List(clause) if !clause.is_empty() => clause,
        _ => {
            return Err(RuntimeError::Message(
                "cond: each clause must be a non-empty list".to_string(),
            ));
        }
    };
    let body = match clause.len() {
        1 => clause[0].clone(),
        _ => Value::List(
            std::iter::once(Value::Symbol("begin".into()))
                .chain(clause.iter().skip(1).cloned())
                .collect(),
        ),
    };
    if matches!(&clause[0], Value::Symbol(name) if name == "else") {
        return Ok(body);
    }
    Ok(Value::list(vec![
        Value::Symbol("if".into()),
        clause[0].clone(),
        body,
        expand_cond(&clauses.skip(1))?,
    ]))
}

#[derive(Clone)]
pub struct Symbol(Rc<str>);

thread_local! {
    static INTERNED_SYMBOLS: RefCell<HashSet<Rc<str>>> = RefCell::new(HashSet::new());
}

impl Symbol {
    pub fn intern(name: &str) -> Self {
        INTERNED_SYMBOLS.with(|symbols| {
            let mut symbols = symbols.borrow_mut();
            if let Some(symbol) = symbols.get(name) {
                return Self(Rc::clone(symbol));
            }

            let symbol: Rc<str> = Rc::from(name);
            symbols.insert(Rc::clone(&symbol));
            Self(symbol)
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Symbol {
    fn from(name: &str) -> Self {
        Self::intern(name)
    }
}

impl From<String> for Symbol {
    fn from(name: String) -> Self {
        Self::intern(&name)
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as *const () as usize).hash(state);
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'world> Value<'world> {
    /// Builds a list value from the elements.
    pub fn list(items: impl Into<List<'world>>) -> Self {
        Value::List(items.into())
    }
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

    pub fn is_entity(&self) -> bool {
        matches!(self, Self::Entity(_))
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
            Self::Entity(entity) => write!(f, "#<entity {} {}>", entity.id(), entity.r#gen().id()),
            Self::Closure {
                function, upvalues, ..
            } => f
                .debug_struct("Closure")
                .field("function", function)
                .field("upvalues", &upvalues.len())
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
            (Self::Entity(l0), Self::Entity(r0)) => l0 == r0,
            _ => false,
        }
    }
}

fn expect_number(val: &Value) -> Result<f64, RuntimeError> {
    match val {
        Value::Number(num) => Ok(*num),
        _ => Err(RuntimeError::Message(format!(
            "expected number, got {:?}",
            val
        ))),
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
                let inner =
                    pair.clone().into_inner().next().ok_or_else(|| {
                        custom_error(&pair, "string without contents".to_string())
                    })?;
                Ok(Value::String(inner.as_str().into()))
            }
            Rule::Ident => Ok(Value::Symbol(pair.as_str().into())),
            Rule::List => Ok(Value::List(
                pair.into_inner()
                    .map(parse_value)
                    .collect::<Result<_, _>>()?,
            )),
            Rule::Quote => {
                let inner =
                    pair.clone().into_inner().next().ok_or_else(|| {
                        custom_error(&pair, "quote without expression".to_string())
                    })?;
                Ok(Value::list(vec![
                    Value::Symbol("quote".into()),
                    parse_value(inner)?,
                ]))
            }
            other => Err(custom_error(&pair, format!("unexpected rule: {:?}", other))),
        }
    }

    let program_pair = SchemeParser::parse(Rule::Program, program)?
        .next()
        .ok_or_else(|| {
            Error::new_from_pos(
                ErrorVariant::CustomError {
                    message: "empty parse result".to_string(),
                },
                pest::Position::from_start(program),
            )
        })?;

    program_pair.into_inner().map(parse_value).collect()
}

#[derive(Debug, Clone)]
pub struct Environment<'world> {
    bindings: HashMap<Symbol, Value<'world>>,
}

impl<'world> Environment<'world> {
    // Break Rc cycles by dropping all bindings. Call after eval to allow WriteStorage borrows to release.
    pub fn clear_bindings(&mut self) {
        self.bindings.clear();
    }
}

type EnvRef<'world> = Rc<RefCell<Environment<'world>>>;

impl<'world> Environment<'world> {
    pub fn set(&mut self, k: impl Into<Symbol>, v: Value<'world>) {
        self.bindings.insert(k.into(), v);
    }

    pub fn get(&self, k: &Symbol) -> Option<Value<'world>> {
        self.bindings.get(k).cloned()
    }
}

#[derive(Debug, Clone)]
pub enum Continuation<'world> {
    ApplyFunction {
        proc: Option<Value<'world>>,
        args: Vec<Expr<'world>>,
        evaluated_args: SmallVec<[Value<'world>; 2]>,
        context: EvalContext<'world>,
    },
    Define {
        target: BindingRef,
        context: EvalContext<'world>,
    },
    Set {
        target: BindingRef,
        context: EvalContext<'world>,
    },
    If {
        consequent: Expr<'world>,
        alternate: Option<Expr<'world>>,
        context: EvalContext<'world>,
    },
    /// Evaluates a sequence, discarding intermediate values, tail-calling the last.
    Sequence {
        expressions: Vec<Expr<'world>>,
        next: usize,
        context: EvalContext<'world>,
    },
    And {
        expressions: Vec<Expr<'world>>,
        next: usize,
        context: EvalContext<'world>,
    },
    Or {
        expressions: Vec<Expr<'world>>,
        next: usize,
        context: EvalContext<'world>,
    },
    FrameReturn {
        frame_index: usize,
    },
}

#[derive(Debug, Clone)]
struct EvalContext<'world> {
    globals: EnvRef<'world>,
    frame: Option<usize>,
    upvalues: Rc<[Upvalue<'world>]>,
}

impl<'world> EvalContext<'world> {
    fn global(globals: EnvRef<'world>) -> Self {
        Self {
            globals,
            frame: None,
            upvalues: Rc::from([]),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallStack<'world> {
    stack: Vec<Continuation<'world>>,
    frames: Vec<ActivationFrame<'world>>,
}

impl<'world> CallStack<'world> {
    pub fn pop_frame(&mut self) -> Result<Continuation<'world>, RuntimeError> {
        self.stack.pop().ok_or_else(|| {
            RuntimeError::Message("internal error: call stack underflow".to_string())
        })
    }

    fn capture(&mut self) -> Self {
        for frame in &mut self.frames {
            for slot in &mut frame.slots {
                slot.promote();
            }
        }
        self.clone()
    }
}

impl<'world> ActivationFrame<'world> {
    fn new(function: &FunctionProto<'world>, arguments: &[Value<'world>]) -> Self {
        let mut slots = vec![Slot::Uninitialized; function.local_count];
        for (&slot, argument) in function.parameter_slots.iter().zip(arguments) {
            slots[slot].write(argument.clone());
        }
        Self { slots }
    }
}

#[derive(Debug)]
enum Thunk<'world> {
    Value(Value<'world>),
    Return(Value<'world>),
    Eval(Expr<'world>, EvalContext<'world>),
    TailCall(Expr<'world>, EvalContext<'world>),
    Call {
        function: Rc<FunctionProto<'world>>,
        arguments: Vec<Value<'world>>,
        globals: EnvRef<'world>,
        upvalues: Rc<[Upvalue<'world>]>,
        caller_frame: Option<usize>,
    },
    InvokeContinuation(Rc<CallStack<'world>>, Value<'world>),
}

impl<'world> Continuation<'world> {
    fn apply_value(
        &mut self,
        val: Value<'world>,
        frames: &mut [ActivationFrame<'world>],
    ) -> Result<Thunk<'world>, RuntimeError> {
        match self {
            Continuation::Define { target, context } => {
                write_binding(target, context, frames, val)?;
                Ok(Thunk::Return(Value::Null))
            }
            Continuation::Set { target, context } => {
                write_binding(target, context, frames, val)?;
                Ok(Thunk::Return(Value::Null))
            }
            Continuation::If {
                consequent,
                alternate,
                context,
            } => {
                if val.is_truthy() {
                    Ok(Thunk::TailCall(consequent.clone(), context.clone()))
                } else if let Some(alt) = alternate {
                    Ok(Thunk::TailCall(alt.clone(), context.clone()))
                } else {
                    Ok(Thunk::Return(Value::Null))
                }
            }
            Continuation::Sequence {
                expressions,
                next,
                context,
            } => {
                if *next >= expressions.len() {
                    Ok(Thunk::Return(val))
                } else {
                    let expression = expressions[*next].clone();
                    *next += 1;
                    if *next == expressions.len() {
                        Ok(Thunk::TailCall(expression, context.clone()))
                    } else {
                        Ok(Thunk::Eval(expression, context.clone()))
                    }
                }
            }
            Continuation::And {
                expressions,
                next,
                context,
            } => {
                if !val.is_truthy() {
                    Ok(Thunk::Return(Value::Bool(false)))
                } else if *next >= expressions.len() {
                    Ok(Thunk::Return(val))
                } else {
                    let expression = expressions[*next].clone();
                    *next += 1;
                    if *next == expressions.len() {
                        Ok(Thunk::TailCall(expression, context.clone()))
                    } else {
                        Ok(Thunk::Eval(expression, context.clone()))
                    }
                }
            }
            Continuation::Or {
                expressions,
                next,
                context,
            } => {
                if val.is_truthy() {
                    Ok(Thunk::Return(val))
                } else if *next >= expressions.len() {
                    Ok(Thunk::Return(Value::Bool(false)))
                } else {
                    let expression = expressions[*next].clone();
                    *next += 1;
                    if *next == expressions.len() {
                        Ok(Thunk::TailCall(expression, context.clone()))
                    } else {
                        Ok(Thunk::Eval(expression, context.clone()))
                    }
                }
            }
            Continuation::ApplyFunction {
                proc,
                args,
                evaluated_args,
                context,
                ..
            } => {
                if proc.is_none() {
                    *proc = Some(val);

                    if args.is_empty() {
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(&func, &[], context)?;
                        Ok(val)
                    } else {
                        let first_arg = args[0].clone();
                        Ok(Thunk::Eval(first_arg, context.clone()))
                    }
                } else {
                    if evaluated_args.len() >= args.len() {
                        return Err(RuntimeError::Message(
                            "internal error: continuation called after it was exhausted"
                                .to_string(),
                        ));
                    }
                    evaluated_args.push(val);
                    if evaluated_args.len() == args.len() {
                        let func = proc.as_ref().unwrap();
                        let val = eval_fn(func, evaluated_args, context)?;
                        Ok(val)
                    } else {
                        let next_arg = args[evaluated_args.len()].clone();
                        Ok(Thunk::Eval(next_arg, context.clone()))
                    }
                }
            }
            Continuation::FrameReturn { .. } => Ok(Thunk::Return(val)),
        }
    }
}

fn write_binding<'world>(
    binding: &BindingRef,
    context: &EvalContext<'world>,
    frames: &mut [ActivationFrame<'world>],
    value: Value<'world>,
) -> Result<(), RuntimeError> {
    match binding {
        BindingRef::Global(name) => context.globals.borrow_mut().set(name.clone(), value),
        BindingRef::Local(slot) => {
            let frame = context
                .frame
                .and_then(|frame| frames.get_mut(frame))
                .ok_or_else(|| RuntimeError::Message("missing local frame".to_string()))?;
            let slot = frame
                .slots
                .get_mut(*slot)
                .ok_or_else(|| RuntimeError::Message("local slot out of bounds".to_string()))?;
            slot.write(value);
        }
        BindingRef::Upvalue(slot) => {
            let upvalue = context
                .upvalues
                .get(*slot)
                .ok_or_else(|| RuntimeError::Message("upvalue out of bounds".to_string()))?;
            *upvalue.borrow_mut() = Some(value);
        }
    }
    Ok(())
}

fn trampoline(thunk: Thunk<'_>) -> Result<Value<'_>, RuntimeError> {
    let mut current_thunk = thunk;
    let mut stack = CallStack {
        stack: vec![],
        frames: vec![],
    };

    loop {
        match current_thunk {
            Thunk::Value(value) => {
                if let Some(continuation) = stack.stack.last_mut() {
                    current_thunk = continuation.apply_value(value, &mut stack.frames)?;
                } else {
                    return Ok(value);
                }
            }
            Thunk::Return(value) => {
                match stack.pop_frame()? {
                    Continuation::FrameReturn { frame_index } => {
                        if frame_index + 1 != stack.frames.len() {
                            return Err(RuntimeError::Message(
                                "call frame stack is out of sync".to_string(),
                            ));
                        }
                        stack.frames.pop();
                    }
                    _ => {}
                }
                current_thunk = Thunk::Value(value);
            }
            Thunk::Eval(expression, context) => {
                current_thunk = eval_expr(expression, context, &mut stack)?;
            }
            Thunk::TailCall(expression, context) => {
                stack.pop_frame()?;
                current_thunk = Thunk::Eval(expression, context);
            }
            Thunk::Call {
                function,
                arguments,
                globals,
                upvalues,
                caller_frame,
            } => {
                if !matches!(stack.pop_frame()?, Continuation::ApplyFunction { .. }) {
                    return Err(RuntimeError::Message(
                        "function call without an application continuation".to_string(),
                    ));
                }

                let tail_call = matches!(
                    (caller_frame, stack.stack.last()),
                    (Some(frame), Some(Continuation::FrameReturn { frame_index })) if frame == *frame_index
                );
                let activation = ActivationFrame::new(&function, &arguments);
                let frame_index = if tail_call {
                    let frame = caller_frame.unwrap();
                    stack.frames[frame] = activation;
                    frame
                } else {
                    let frame = stack.frames.len();
                    stack.frames.push(activation);
                    stack
                        .stack
                        .push(Continuation::FrameReturn { frame_index: frame });
                    frame
                };
                let context = EvalContext {
                    globals,
                    frame: Some(frame_index),
                    upvalues,
                };
                current_thunk = Thunk::Eval((*function.body).clone(), context);
            }
            Thunk::InvokeContinuation(saved_stack, value) => {
                stack = (*saved_stack).clone();
                current_thunk = Thunk::Value(value);
            }
        }
    }
}

fn eval_expr<'world>(
    expression: Expr<'world>,
    context: EvalContext<'world>,
    stack: &mut CallStack<'world>,
) -> Result<Thunk<'world>, RuntimeError> {
    match expression {
        Expr::Literal(value) => Ok(Thunk::Value(value)),
        Expr::Global(name) => context
            .globals
            .borrow()
            .get(&name)
            .map(Thunk::Value)
            .ok_or_else(|| RuntimeError::Message(format!("Undefined identifier: {:?}", name))),
        Expr::Local(slot) => {
            let value = context
                .frame
                .and_then(|frame| stack.frames.get(frame))
                .and_then(|frame| frame.slots.get(slot))
                .and_then(Slot::read)
                .ok_or_else(|| {
                    RuntimeError::Message(format!("uninitialized local slot {}", slot))
                })?;
            Ok(Thunk::Value(value))
        }
        Expr::Upvalue(slot) => context
            .upvalues
            .get(slot)
            .and_then(|value| value.borrow().clone())
            .map(Thunk::Value)
            .ok_or_else(|| RuntimeError::Message(format!("uninitialized upvalue {}", slot))),
        Expr::Define(target, value) => {
            stack.stack.push(Continuation::Define {
                target,
                context: context.clone(),
            });
            Ok(Thunk::Eval(*value, context))
        }
        Expr::Set(target, value) => {
            stack.stack.push(Continuation::Set {
                target,
                context: context.clone(),
            });
            Ok(Thunk::Eval(*value, context))
        }
        Expr::If(test, consequent, alternate) => {
            stack.stack.push(Continuation::If {
                consequent: *consequent,
                alternate: alternate.map(|value| *value),
                context: context.clone(),
            });
            Ok(Thunk::Eval(*test, context))
        }
        Expr::Begin(expressions) => match expressions.len() {
            0 => Ok(Thunk::Value(Value::Null)),
            1 => Ok(Thunk::Eval(
                expressions.into_iter().next().unwrap(),
                context,
            )),
            _ => {
                let first = expressions[0].clone();
                stack.stack.push(Continuation::Sequence {
                    expressions,
                    next: 1,
                    context: context.clone(),
                });
                Ok(Thunk::Eval(first, context))
            }
        },
        Expr::And(expressions) => {
            if expressions.is_empty() {
                return Ok(Thunk::Value(Value::Bool(true)));
            }
            let first = expressions[0].clone();
            if expressions.len() == 1 {
                Ok(Thunk::Eval(first, context))
            } else {
                stack.stack.push(Continuation::And {
                    expressions,
                    next: 1,
                    context: context.clone(),
                });
                Ok(Thunk::Eval(first, context))
            }
        }
        Expr::Or(expressions) => {
            if expressions.is_empty() {
                return Ok(Thunk::Value(Value::Bool(false)));
            }
            let first = expressions[0].clone();
            if expressions.len() == 1 {
                Ok(Thunk::Eval(first, context))
            } else {
                stack.stack.push(Continuation::Or {
                    expressions,
                    next: 1,
                    context: context.clone(),
                });
                Ok(Thunk::Eval(first, context))
            }
        }
        Expr::Lambda(function) => {
            let mut upvalues = Vec::with_capacity(function.captures.len());
            for source in &function.captures {
                let upvalue = match *source {
                    CaptureSource::Local(slot) => {
                        let frame = context
                            .frame
                            .and_then(|frame| stack.frames.get_mut(frame))
                            .ok_or_else(|| {
                                RuntimeError::Message("missing capture frame".to_string())
                            })?;
                        frame
                            .slots
                            .get_mut(slot)
                            .ok_or_else(|| {
                                RuntimeError::Message("capture slot out of bounds".to_string())
                            })?
                            .promote()
                    }
                    CaptureSource::Upvalue(slot) => {
                        context.upvalues.get(slot).cloned().ok_or_else(|| {
                            RuntimeError::Message("capture upvalue out of bounds".to_string())
                        })?
                    }
                };
                upvalues.push(upvalue);
            }
            Ok(Thunk::Value(Value::Closure {
                globals: Rc::clone(&context.globals),
                function,
                upvalues: upvalues.into(),
            }))
        }
        Expr::Call(function, arguments) => {
            let first = function.as_ref().clone();
            stack.stack.push(Continuation::ApplyFunction {
                proc: None,
                args: arguments,
                evaluated_args: SmallVec::new(),
                context: context.clone(),
            });
            Ok(Thunk::Eval(first, context))
        }
        Expr::CallCc(callback) => {
            let continuation = Value::Continuation(Rc::new(stack.capture()));
            stack.stack.push(Continuation::ApplyFunction {
                proc: None,
                args: vec![Expr::Literal(continuation)],
                evaluated_args: SmallVec::new(),
                context: context.clone(),
            });
            Ok(Thunk::Eval(*callback, context))
        }
    }
}

fn eval_fn<'world>(
    function: &Value<'world>,
    arguments: &[Value<'world>],
    caller: &EvalContext<'world>,
) -> Result<Thunk<'world>, RuntimeError> {
    match function {
        Value::NativeFunc(f) => Ok(Thunk::Return(f(arguments)?)),
        Value::NativeMethod(f) => Ok(Thunk::Return(f(arguments)?)),
        Value::Closure {
            globals,
            function,
            upvalues,
        } => Ok(Thunk::Call {
            function: Rc::clone(function),
            arguments: arguments.to_vec(),
            globals: Rc::clone(globals),
            upvalues: Rc::clone(upvalues),
            caller_frame: caller.frame,
        }),
        Value::Continuation(saved_stack) => {
            let value = match arguments {
                [] => Value::Null,
                [value] => value.clone(),
                _ => {
                    return Err(RuntimeError::Message(format!(
                        "continuation takes at most 1 argument, got {}",
                        arguments.len()
                    )));
                }
            };
            Ok(Thunk::InvokeContinuation(Rc::clone(saved_stack), value))
        }
        value => Err(RuntimeError::Message(format!(
            "Expected function, got {:?}",
            value
        ))),
    }
}

pub fn eval<'world>(
    expr: Value<'world>,
    globals: EnvRef<'world>,
) -> Result<Value<'world>, RuntimeError> {
    let mut compiler = FunctionCompiler::top_level();
    let expression = compile_expr(&expr, &mut compiler)?;
    trampoline(Thunk::Eval(expression, EvalContext::global(globals)))
}

#[cfg(test)]
pub fn parse_and_eval(program: &str) -> Option<Value<'_>> {
    let parsed: Vec<Value<'_>> = parse_lscheme(program).unwrap();
    let env = default_environment().unwrap();
    let mut result = None;
    for form in parsed {
        result = Some(eval(form, Rc::clone(&env)).unwrap());
    }
    result
}

#[cfg(any())]
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

#[cfg(any())]
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
                                _ => {
                                    return Err(RuntimeError::Message(
                                        "define: function name must be a symbol".to_string(),
                                    ));
                                }
                            };
                            let params = Value::List(func_spec.skip(1));
                            let lambda = Value::List(
                                std::iter::once(Value::Symbol("lambda".into()))
                                    .chain(std::iter::once(params))
                                    .chain(list.iter().skip(2).cloned())
                                    .collect(),
                            );
                            let desugared = Value::list(vec![
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
                        _ => {
                            return Err(RuntimeError::Message(
                                "set! first argument must be a symbol".to_string(),
                            ));
                        }
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
                            .collect::<Result<Vec<_>, _>>()?
                            .into(),
                        _ => {
                            return Err(RuntimeError::Message(
                                "lambda second argument must be a list of symbols".to_string(),
                            ));
                        }
                    };
                    let body = if list.len() == 3 {
                        list[2].clone()
                    } else {
                        Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list.iter().skip(2).cloned())
                                .collect(),
                        )
                    };
                    let body = Rc::new(body);
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
                    let exprs = list.skip(1);
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Null));
                    }
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(exprs[0].clone(), env));
                    }
                    let first = exprs[0].clone();
                    let rest = exprs.skip(1);
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
                        _ => {
                            return Err(RuntimeError::Message(
                                "let: bindings must be a list".to_string(),
                            ));
                        }
                    };
                    let mut params = Vec::new();
                    let mut vals = Vec::new();
                    for binding in &bindings {
                        match binding {
                            Value::List(pair) if pair.len() == 2 => {
                                params.push(pair[0].clone());
                                vals.push(pair[1].clone());
                            }
                            _ => {
                                return Err(RuntimeError::Message(
                                    "let: each binding must be a (name value) pair".to_string(),
                                ));
                            }
                        }
                    }
                    let lambda = Value::List(
                        std::iter::once(Value::Symbol("lambda".into()))
                            .chain(std::iter::once(Value::list(params)))
                            .chain(list.iter().skip(2).cloned())
                            .collect(),
                    );
                    let application = Value::List(std::iter::once(lambda).chain(vals).collect());
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
                        _ => {
                            return Err(RuntimeError::Message(
                                "let*: bindings must be a list".to_string(),
                            ));
                        }
                    };
                    if bindings.is_empty() {
                        let begin_expr = Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list.iter().skip(2).cloned())
                                .collect(),
                        );
                        Ok(Thunk::Eval(begin_expr, env))
                    } else {
                        let first_binding = Value::list(vec![bindings[0].clone()]);
                        let rest_bindings = Value::List(bindings.skip(1));
                        let inner = Value::List(
                            std::iter::once(Value::Symbol("let*".into()))
                                .chain(std::iter::once(rest_bindings))
                                .chain(list.iter().skip(2).cloned())
                                .collect(),
                        );
                        let outer =
                            Value::list(vec![Value::Symbol("let".into()), first_binding, inner]);
                        Ok(Thunk::Eval(outer, env))
                    }
                }
                Value::Symbol(x) if x == "and" => {
                    let exprs = list.skip(1);
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Bool(true)));
                    }
                    let first = exprs[0].clone();
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(first, env));
                    }
                    let rest = exprs.skip(1);
                    stack.push_frame(Continuation::And {
                        remaining: rest,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(first, env))
                }
                Value::Symbol(x) if x == "or" => {
                    let exprs = list.skip(1);
                    if exprs.is_empty() {
                        return Ok(Thunk::Value(Value::Bool(false)));
                    }
                    let first = exprs[0].clone();
                    if exprs.len() == 1 {
                        return Ok(Thunk::Eval(first, env));
                    }
                    let rest = exprs.skip(1);
                    stack.push_frame(Continuation::Or {
                        remaining: rest,
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(first, env))
                }
                Value::Symbol(x) if x == "cond" => {
                    fn desugar_cond<'w>(clauses: &List<'w>) -> Result<Value<'w>, RuntimeError> {
                        if clauses.is_empty() {
                            return Ok(Value::Null);
                        }
                        let clause = match &clauses[0] {
                            Value::List(l) if !l.is_empty() => l,
                            _ => {
                                return Err(RuntimeError::Message(
                                    "cond: each clause must be a non-empty list".to_string(),
                                ));
                            }
                        };
                        let test = &clause[0];
                        let body = if clause.len() == 2 {
                            clause[1].clone()
                        } else {
                            Value::List(
                                std::iter::once(Value::Symbol("begin".into()))
                                    .chain(clause.iter().skip(1).cloned())
                                    .collect(),
                            )
                        };
                        if matches!(test, Value::Symbol(s) if s == "else") {
                            Ok(body)
                        } else {
                            let rest = desugar_cond(&clauses.skip(1))?;
                            Ok(Value::list(vec![
                                Value::Symbol("if".into()),
                                test.clone(),
                                body,
                                rest,
                            ]))
                        }
                    }
                    let desugared = desugar_cond(&list.skip(1))?;
                    Ok(Thunk::Eval(desugared, env))
                }
                Value::Symbol(x) if x == "when" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "when requires test and body".to_string(),
                        ));
                    }
                    let test = list[1].clone();
                    let body = if list.len() == 3 {
                        list[2].clone()
                    } else {
                        Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list.iter().skip(2).cloned())
                                .collect(),
                        )
                    };
                    let if_expr = Value::list(vec![Value::Symbol("if".into()), test, body]);
                    Ok(Thunk::Eval(if_expr, env))
                }
                Value::Symbol(x) if x == "unless" => {
                    if list.len() < 3 {
                        return Err(RuntimeError::Message(
                            "unless requires test and body".to_string(),
                        ));
                    }
                    let test = list[1].clone();
                    let body = if list.len() == 3 {
                        list[2].clone()
                    } else {
                        Value::List(
                            std::iter::once(Value::Symbol("begin".into()))
                                .chain(list.iter().skip(2).cloned())
                                .collect(),
                        )
                    };
                    let not_test = Value::list(vec![Value::Symbol("not".into()), test]);
                    let if_expr = Value::list(vec![Value::Symbol("if".into()), not_test, body]);
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
                        args: List::unit(captured_k),
                        evaluated_args: SmallVec::new(),
                        env: Rc::clone(&env),
                    });
                    Ok(Thunk::Eval(list[1].clone(), env))
                }
                head => {
                    stack.push_frame(Continuation::ApplyFunction {
                        proc: None,
                        args: list.skip(1),
                        evaluated_args: SmallVec::new(),
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

#[cfg(any())]
pub fn eval<'world>(
    expr: Value<'world>,
    env: EnvRef<'world>,
) -> Result<Value<'world>, RuntimeError> {
    trampoline(Thunk::Eval(expr, env))
}

#[cfg(any())]
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
#[cfg(any())]
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
            let mut bindings = Bindings::Local(SmallVec::new());
            for (arg_name, evaluated_arg) in arg_names.iter().zip(args) {
                bindings.insert(arg_name.clone(), evaluated_arg.clone());
            }
            let env_ref = Rc::new(RefCell::new(Environment {
                bindings,
                parent: Some(Rc::clone(&lambda_env)),
            }));
            Ok(Thunk::TailCall((**body).clone(), env_ref))
        }
        Value::Continuation(saved_stack) => {
            let arg = match args {
                [] => Value::Null,
                [arg] => arg.clone(),
                _ => {
                    return Err(RuntimeError::Message(format!(
                        "continuation takes at most 1 argument, got {}",
                        args.len()
                    )));
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
        let expected = Ok(vec![Value::Symbol("foo".into())]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn symbols_are_interned() {
        let first = Symbol::intern("shared-name");
        let second = Symbol::intern(&"shared-name".to_string());

        assert!(Rc::ptr_eq(&first.0, &second.0));
        let mut values = HashMap::new();
        values.insert(first, 42);
        assert_eq!(values.get(&second), Some(&42));
    }

    #[test]
    fn test_parse_simple_list() {
        let program = "(+ 1 2)";
        let expected = Ok(vec![Value::list(vec![
            Value::Symbol("+".into()),
            Value::Number(1.0),
            Value::Number(2.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_nested_list() {
        let program = "(define x (list 1 2))";
        let expected = Ok(vec![Value::list(vec![
            Value::Symbol("define".into()),
            Value::Symbol("x".into()),
            Value::list(vec![
                Value::Symbol("list".into()),
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
            Value::list(vec![
                Value::Symbol("+".into()),
                Value::Number(1.0),
                Value::Number(2.0),
            ]),
            Value::list(vec![
                Value::Symbol("define".into()),
                Value::Symbol("a".into()),
                Value::Number(10.0),
            ]),
        ]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_empty_list() {
        let program = "()";
        let expected = Ok(vec![Value::list(vec![])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_list_with_mixed_types() {
        let program = r#"(set! my-var "hello" 123)"#;
        let expected = Ok(vec![Value::list(vec![
            Value::Symbol("set!".into()),
            Value::Symbol("my-var".into()),
            Value::String("hello".to_string()),
            Value::Number(123.0),
        ])]);
        assert_eq!(parse_lscheme(program), expected);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let program = "  ( +   1   2   )  ";
        let expected = Ok(vec![Value::list(vec![
            Value::Symbol("+".into()),
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
    fn lexical_resolution_respects_shadowing() {
        let result = parse_and_eval("((lambda (x) (let ((x 4)) (+ x 1))) 2)").unwrap();
        assert_eq!(result, Value::Number(5.0));
    }

    #[test]
    fn recursive_internal_definition_uses_its_local_slot() {
        let result = parse_and_eval(
            "((lambda ()
                (define (sum-down n)
                  (if (= n 0) 0 (+ n (sum-down (- n 1)))))
                (sum-down 10)))",
        )
        .unwrap();
        assert_eq!(result, Value::Number(55.0));
    }

    #[test]
    fn tail_calls_reuse_activation_frames() {
        let result = parse_and_eval(
            "((lambda ()
                (define (count-down n)
                  (if (= n 0) n (count-down (- n 1))))
                (count-down 2000)))",
        )
        .unwrap();
        assert_eq!(result, Value::Number(0.0));
    }

    #[test]
    fn closure_clones_share_their_body() {
        let closure = parse_and_eval("(lambda (x) (+ x 1))").unwrap();
        let cloned = closure.clone();
        let (
            Value::Closure { function, .. },
            Value::Closure {
                function: cloned_function,
                ..
            },
        ) = (&closure, &cloned)
        else {
            panic!("expected closures");
        };

        assert!(Rc::ptr_eq(function, cloned_function));
    }

    #[test]
    fn function_frames_spill_after_two_arguments() {
        let result = parse_and_eval("((lambda (a b c) (+ a b c)) 1 2 3)").unwrap();
        assert_eq!(result, Value::Number(6.0));
    }

    #[test]
    fn function_application_arguments_spill_after_two() {
        let result = parse_and_eval("((lambda (a b c) (+ a b c)) 1 2 3)").unwrap();
        assert_eq!(result, Value::Number(6.0));
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
        assert_eq!(parse_and_eval("'foo").unwrap(), Value::Symbol("foo".into()));
        assert_eq!(
            parse_and_eval("(quote foo)").unwrap(),
            Value::Symbol("foo".into())
        );
        assert_eq!(
            parse_and_eval("'(1 2 3)").unwrap(),
            Value::list(vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Number(3.0)
            ])
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
    fn set_updates_a_captured_local_binding() {
        let result = parse_and_eval(
            "(define make-counter (lambda (x) (lambda () (set! x (+ x 1)) x)))
             (define counter (make-counter 0))
             (list (counter) (counter))",
        )
        .unwrap();
        assert_eq!(
            result,
            Value::list(vec![Value::Number(1.0), Value::Number(2.0)])
        );
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
        assert_eq!(
            parse_and_eval("(car '(1 2 3))").unwrap(),
            Value::Number(1.0)
        );
        assert_eq!(
            parse_and_eval("(cdr '(1 2 3))").unwrap(),
            Value::list(vec![Value::Number(2.0), Value::Number(3.0)])
        );
        assert_eq!(
            parse_and_eval("(cons 1 '(2 3))").unwrap(),
            Value::list(vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Number(3.0)
            ])
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
        assert!(matches!(
            eval_err("(undefined-fn 1)"),
            RuntimeError::Message(_)
        ));
        assert!(matches!(eval_err("(car '())"), RuntimeError::Message(_)));
        assert!(matches!(
            eval_err("(call/cc (lambda (k) (k 1 2)))"),
            RuntimeError::Message(_)
        ));
    }

    #[test]
    fn test_error_in_callback_propagates() {
        // errors raised inside nested evaluation reach the caller
        assert!(matches!(
            eval_err("((lambda () (+ 1 'a)))"),
            RuntimeError::Message(_)
        ));
    }
}
