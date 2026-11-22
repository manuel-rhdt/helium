//! Math functions: `calc()`, `min()`, `max()`, `clamp()`, `round()`, `mod()`, `rem()`, the
//! trigonometric functions, `pow()`, `sqrt()`, `hypot()`, `log()`, `exp()`, `abs()` and `sign()`.
//! https://drafts.csswg.org/css-values-4/#math
//!
//! A math function is parsed into a tree of [`Node`]s and type-checked as the specification
//! prescribes (the types of CSS Typed OM, with the percent hint), so that, for one, `calc(1px * 2px
//! / 1em)` is a length and `calc(1px + 2)` is not valid. The leaves keep their units until the
//! computed value is found, when the font-relative and viewport-relative lengths become pixels.
//! A computed value whose percentages appear in a linear combination with lengths becomes a
//! [`LengthPercentage`] of a length and a percentage; any other one (`min(100%, 40em)`) keeps the
//! tree, which layout evaluates once the size that the percentages refer to is known.
//!
//! The trees are interned and live as long as the program: a [`Calc`] is a reference to one, so that
//! the values of properties stay `Copy`, and so that Taffy, which holds a `calc()` as an untyped
//! pointer, can be handed one. The trees of the specified values are as many as the math functions
//! of the style sheets; those of the computed values are shared between the elements whose
//! relative units come to the same pixels.
//!
//! Numbers are computed with `f32`, in the canonical units: pixels, radians, and percentages as
//! fractions of the size they refer to. Every leaf is then a plain number, and the functions can be
//! evaluated without the types, which were checked when the function was parsed.

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

use super::computed::{FontSizes, LengthPercentage, PixelLength};
use super::specified::{Percentage, Unit};
use crate::style::parse::StyleParseError;

/// Values are clamped to this, in their canonical unit, when a top-level calculation is infinite:
/// what browsers use as the largest length.
const LIMIT: f32 = 33_554_428.0;

/// An `f32` that is compared and hashed by its bits, so that trees can be interned (`NaN` equals
/// itself, `0` and `-0` are different, which `sign()` and division can tell apart).
#[derive(Clone, Copy)]
pub struct Bits(pub f32);

impl PartialEq for Bits {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Bits {}

impl Hash for Bits {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state)
    }
}

impl fmt::Debug for Bits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// How `round()` rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rounding {
    Nearest,
    Up,
    Down,
    ToZero,
}

/// A math function other than `calc()`, whose arguments are the children of a
/// [`Node::Function`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathFunction {
    Min,
    Max,
    /// The arguments are the minimum, the value and the maximum; `none` is an infinite number.
    Clamp,
    /// The arguments are the value and the step (1 if it was left out).
    Round(Rounding),
    Mod,
    Rem,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Pow,
    Sqrt,
    Hypot,
    /// The arguments are the value and, if there is one, the base.
    Log,
    Exp,
    Abs,
    Sign,
}

/// A node of a calculation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Number(Bits),
    /// A percentage, as a fraction (`50%` is 0.5).
    Percentage(Bits),
    /// A length in one of the units. In a computed value, all lengths are in pixels.
    Length(Bits, Unit),
    /// An angle, in radians.
    Angle(Bits),
    Sum(Box<[Node]>),
    Product(Box<[Node]>),
    Negate(Box<Node>),
    /// `1 / x`.
    Invert(Box<Node>),
    Function(MathFunction, Box<[Node]>),
}

/// The range of values that a property allows, to which the result of a calculation is clamped
/// (a calculation that is out of the range is not invalid, unlike a literal value).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Range {
    #[default]
    All,
    NonNegative,
}

impl Range {
    fn clamp(self, value: f32) -> f32 {
        match self {
            Range::All => value,
            // `max` turns `-0` into `0` too
            Range::NonNegative => value.max(0.0),
        }
    }
}

/// A calculation and the range of the property it is the value of.
// 8-byte aligned, because Taffy keeps its own tag in the 3 low bits of a `calc()` pointer.
#[repr(align(8))]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Expr {
    pub node: Node,
    pub range: Range,
}

/// A reference to an interned calculation: a top-level math function.
#[derive(Clone, Copy)]
pub struct Calc(&'static Expr);

impl PartialEq for Calc {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0) || self.0 == other.0
    }
}

impl fmt::Debug for Calc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl fmt::Display for Calc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0.node {
            node @ Node::Function(..) => write!(f, "{}", node),
            node => write!(f, "calc({})", node),
        }
    }
}

static INTERNED: LazyLock<Mutex<HashSet<&'static Expr>>> = LazyLock::new(Default::default);

impl Calc {
    pub fn new(node: Node, range: Range) -> Calc {
        let expr = Expr { node, range };
        let mut interned = INTERNED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = interned.get(&expr) {
            return Calc(existing);
        }
        let leaked: &'static Expr = Box::leak(Box::new(expr));
        interned.insert(leaked);
        Calc(leaked)
    }

    /// `length + percentage`, for Taffy, which has no other way to hold the sum.
    pub fn linear(length: PixelLength, percentage: Percentage) -> Calc {
        Calc::new(
            Node::Sum(Box::new([Node::Length(Bits(length.0), Unit::Px), Node::Percentage(Bits(percentage.0))])),
            Range::All,
        )
    }

    pub fn node(self) -> &'static Node {
        &self.0.node
    }

    pub fn range(self) -> Range {
        self.0.range
    }

    /// The same calculation, clamped to another range.
    pub fn with_range(self, range: Range) -> Calc {
        if self.0.range == range { self } else { Calc::new(self.0.node.clone(), range) }
    }

    /// The pointer that Taffy holds for a `calc()` value.
    pub fn as_ptr(self) -> *const () {
        self.0 as *const Expr as *const ()
    }

    /// The calculation of a pointer that [`Calc::as_ptr`] made.
    ///
    /// # Safety
    /// `ptr` must come from [`Calc::as_ptr`]. The calculations are never freed, so such a pointer
    /// is always valid.
    pub unsafe fn from_ptr(ptr: *const ()) -> Calc {
        Calc(unsafe { &*(ptr as *const Expr) })
    }

    pub fn contains_percentage(self) -> bool {
        self.0.node.contains_percentage()
    }

    /// The value of a computed calculation, with percentages of `basis`, in its canonical unit,
    /// censored (`NaN` is 0, infinity is the largest value) and clamped to the range of the
    /// property.
    pub fn resolve(self, basis: f32) -> f32 {
        self.0.range.clamp(censor(self.0.node.eval(basis)))
    }

    /// The value of a calculation that has no relative lengths, or of a computed calculation,
    /// where nothing gives percentages a size to refer to: they can only be there if they cancel
    /// out (`calc(2px * (50% / 25%))` is `4px`), so they are fractions.
    pub fn value(self) -> f32 {
        self.resolve(1.0)
    }

    /// The calculation with the relative lengths in pixels.
    fn absolutize(self, sizes: FontSizes) -> Calc {
        if !self.0.node.has_relative_length() {
            return self;
        }
        Calc::new(self.0.node.absolutize(sizes), self.0.range)
    }

    /// The computed value of a `<length>`: pixels.
    pub fn to_px(self, sizes: FontSizes) -> PixelLength {
        PixelLength(self.absolutize(sizes).value())
    }

    /// The value of a `<number>`, which may divide lengths (`calc(1em / 1px)`).
    pub fn to_number(self, sizes: FontSizes) -> f32 {
        self.absolutize(sizes).value()
    }

    /// The computed value of a `<length-percentage>` whose percentages refer to a size that is
    /// known now (`font-size`, say, whose percentages are of the font size of the parent).
    pub fn to_px_with_basis(self, sizes: FontSizes, basis: PixelLength) -> PixelLength {
        PixelLength(self.absolutize(sizes).resolve(basis.0))
    }

    /// The computed value of a `<length-percentage>`: a length, a percentage, both, or a
    /// calculation if the percentage is not a term of a sum.
    pub fn to_length_percentage(self, sizes: FontSizes) -> LengthPercentage {
        let computed = self.absolutize(sizes);
        if !computed.contains_percentage() {
            return LengthPercentage::PixelLength(PixelLength(computed.value()));
        }
        let non_negative = computed.range() == Range::NonNegative;
        match computed.node().linear() {
            Some((length, fraction)) if length.is_finite() && fraction.is_finite() => {
                match (length, fraction) {
                    (length, fraction) if non_negative && (length < 0.0 || fraction < 0.0) => {
                        // only the sum is clamped: that needs the size
                        LengthPercentage::Calc(computed)
                    }
                    (length, fraction) if length == 0.0 => LengthPercentage::Percentage(Percentage(fraction)),
                    (length, fraction) if fraction == 0.0 => LengthPercentage::PixelLength(PixelLength(length)),
                    (length, fraction) => LengthPercentage::Both {
                        length: PixelLength(length),
                        percentage: Percentage(fraction),
                    },
                }
            }
            _ => LengthPercentage::Calc(computed),
        }
    }

    /// `100% - self`, for `background-position: right 10px`.
    pub fn complement(self) -> Calc {
        Calc::new(
            Node::Sum(Box::new([Node::Percentage(Bits(1.0)), Node::Negate(Box::new(self.0.node.clone()))])),
            self.0.range,
        )
    }
}

/// What a top-level calculation is when it is not a number: `NaN` is 0 and the infinities are
/// the largest values.
fn censor(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value.clamp(-LIMIT, LIMIT) }
}

impl Node {
    fn children(&self) -> &[Node] {
        match self {
            Node::Number(_) | Node::Percentage(_) | Node::Length(..) | Node::Angle(_) => &[],
            Node::Sum(children) | Node::Product(children) | Node::Function(_, children) => children,
            Node::Negate(child) | Node::Invert(child) => std::slice::from_ref(&**child),
        }
    }

    pub fn contains_percentage(&self) -> bool {
        matches!(self, Node::Percentage(_)) || self.children().iter().any(Node::contains_percentage)
    }

    fn has_relative_length(&self) -> bool {
        matches!(self, Node::Length(_, unit) if *unit != Unit::Px)
            || self.children().iter().any(Node::has_relative_length)
    }

    /// The tree with all lengths in pixels.
    fn absolutize(&self, sizes: FontSizes) -> Node {
        let map = |children: &[Node]| children.iter().map(|child| child.absolutize(sizes)).collect();
        match self {
            &Node::Length(Bits(value), unit) => {
                Node::Length(Bits(PixelLength::from_val_unit(value, unit, sizes).0), Unit::Px)
            }
            Node::Number(_) | Node::Percentage(_) | Node::Angle(_) => self.clone(),
            Node::Sum(children) => Node::Sum(map(children)),
            Node::Product(children) => Node::Product(map(children)),
            Node::Negate(child) => Node::Negate(Box::new(child.absolutize(sizes))),
            Node::Invert(child) => Node::Invert(Box::new(child.absolutize(sizes))),
            Node::Function(function, children) => Node::Function(*function, map(children)),
        }
    }

    /// The tree as `length + fraction * basis`, if it is linear in the percentages. The lengths
    /// must be in pixels.
    fn linear(&self) -> Option<(f32, f32)> {
        if !self.contains_percentage() {
            return Some((self.eval(0.0), 0.0));
        }
        match self {
            &Node::Percentage(Bits(fraction)) => Some((0.0, fraction)),
            Node::Sum(children) => children.iter().try_fold((0.0, 0.0), |(length, fraction), child| {
                let (l, f) = child.linear()?;
                Some((length + l, fraction + f))
            }),
            Node::Negate(child) => child.linear().map(|(length, fraction)| (-length, -fraction)),
            Node::Product(children) => {
                // linear if only one factor has percentages: the others are constants
                let mut variable = None;
                let mut factor = 1.0;
                for child in children.iter() {
                    if child.contains_percentage() {
                        if variable.replace(child).is_some() {
                            return None;
                        }
                    } else {
                        factor *= child.eval(0.0);
                    }
                }
                let (length, fraction) = variable?.linear()?;
                Some((length * factor, fraction * factor))
            }
            _ => None,
        }
    }

    /// The value of the tree, in canonical units, with percentages of `basis`. The lengths must
    /// be in pixels.
    pub fn eval(&self, basis: f32) -> f32 {
        match self {
            Node::Number(Bits(value)) | Node::Angle(Bits(value)) => *value,
            Node::Percentage(Bits(fraction)) => fraction * basis,
            &Node::Length(Bits(value), unit) => {
                debug_assert!(unit == Unit::Px || is_absolute(unit), "a relative length is evaluated");
                PixelLength::from_val_unit(value, unit, FontSizes::default()).0
            }
            Node::Sum(children) => children.iter().map(|child| child.eval(basis)).sum(),
            Node::Product(children) => children.iter().map(|child| child.eval(basis)).product(),
            Node::Negate(child) => -child.eval(basis),
            Node::Invert(child) => 1.0 / child.eval(basis),
            Node::Function(function, children) => {
                let args: Vec<f32> = children.iter().map(|child| child.eval(basis)).collect();
                apply(*function, &args)
            }
        }
    }
}

/// The value of a math function of the values of its arguments.
/// https://drafts.csswg.org/css-values-4/#math
fn apply(function: MathFunction, args: &[f32]) -> f32 {
    use MathFunction::*;
    let arg = |i: usize| args.get(i).copied().unwrap_or(f32::NAN);
    match function {
        // `NaN` in any argument makes the result `NaN` (`f32::min` would ignore it)
        Min => args.iter().copied().reduce(|a, b| if a.is_nan() || b.is_nan() { f32::NAN } else { min(a, b) }).unwrap_or(f32::NAN),
        Max => args.iter().copied().reduce(|a, b| if a.is_nan() || b.is_nan() { f32::NAN } else { max(a, b) }).unwrap_or(f32::NAN),
        Clamp => {
            let (lower, value, upper) = (arg(0), arg(1), arg(2));
            if lower.is_nan() || value.is_nan() || upper.is_nan() {
                f32::NAN
            } else {
                // the minimum wins if it is larger than the maximum
                max(lower, min(value, upper))
            }
        }
        Round(strategy) => round(strategy, arg(0), arg(1)),
        Mod | Rem => {
            let (a, b) = (arg(0), arg(1));
            if b == 0.0 || a.is_infinite() || a.is_nan() || b.is_nan() {
                f32::NAN
            } else if b.is_infinite() {
                // `mod()` takes the sign of `b`: `a` is the result only if it has that sign
                // (a zero of the other sign too)
                if function == Mod && a.is_sign_negative() != b.is_sign_negative() {
                    f32::NAN
                } else {
                    a
                }
            } else {
                let remainder = a % b;
                if function == Mod && remainder != 0.0 && (remainder < 0.0) != (b < 0.0) {
                    remainder + b
                } else {
                    remainder
                }
            }
        }
        Sin => arg(0).sin(),
        Cos => arg(0).cos(),
        Tan => arg(0).tan(),
        Asin => arg(0).asin(),
        Acos => arg(0).acos(),
        Atan => arg(0).atan(),
        Atan2 => arg(0).atan2(arg(1)),
        Pow => arg(0).powf(arg(1)),
        Sqrt => arg(0).sqrt(),
        Hypot => {
            if args.iter().any(|a| a.is_infinite()) {
                f32::INFINITY
            } else {
                args.iter().map(|a| a * a).sum::<f32>().sqrt()
            }
        }
        Log => match args.get(1) {
            Some(&base) => arg(0).ln() / base.ln(),
            None => arg(0).ln(),
        },
        Exp => arg(0).exp(),
        Abs => arg(0).abs(),
        Sign => {
            let a = arg(0);
            if a == 0.0 || a.is_nan() { a } else { a.signum() }
        }
    }
}

/// `min()` of two numbers that are not `NaN`, where `-0` is less than `0`.
fn min(a: f32, b: f32) -> f32 {
    if a < b || (a == b && a.is_sign_negative()) { a } else { b }
}

fn max(a: f32, b: f32) -> f32 {
    if a > b || (a == b && a.is_sign_positive()) { a } else { b }
}

/// `round(<strategy>, a, b)`.
/// https://drafts.csswg.org/css-values-4/#round-func
fn round(strategy: Rounding, a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() || b == 0.0 || (a.is_infinite() && b.is_infinite()) {
        return f32::NAN;
    }
    if a.is_infinite() {
        return a;
    }
    if b.is_infinite() {
        return match strategy {
            Rounding::Nearest | Rounding::ToZero => 0.0f32.copysign(a),
            Rounding::Up if a > 0.0 => f32::INFINITY,
            Rounding::Up => 0.0f32.copysign(a),
            Rounding::Down if a < 0.0 => f32::NEG_INFINITY,
            Rounding::Down => 0.0f32.copysign(a),
        };
    }
    let step = b.abs();
    let quotient = a / step;
    if quotient == quotient.trunc() {
        // a multiple of the step is itself (with its sign, if it is zero)
        return a;
    }
    // a zero below is `0`, a zero above is `-0`
    let lower = (quotient.floor() * step) + 0.0;
    let upper = {
        let upper = quotient.ceil() * step;
        if upper == 0.0 { -0.0 } else { upper }
    };
    match strategy {
        Rounding::Nearest => {
            if a - lower < upper - a { lower } else { upper }
        }
        Rounding::Up => upper,
        Rounding::Down => lower,
        Rounding::ToZero => {
            if a > 0.0 { lower } else { upper }
        }
    }
}

fn is_absolute(unit: Unit) -> bool {
    matches!(unit, Unit::Px | Unit::Cm | Unit::Mm | Unit::In | Unit::Pt | Unit::Pc)
}

// --- types ---

/// A base type of the CSS types that the values here can have, other than "percent" (which a
/// percent hint cannot be: [`Type::percent`] is its power).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Length,
    Angle,
}

/// The type of a calculation: the power of each base type, and the percent hint (the type that the
/// percentages, which have taken it, refer to).
/// https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-type
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Type {
    length: i8,
    angle: i8,
    percent: i8,
    hint: Option<Base>,
}

impl Type {
    const NUMBER: Type = Type { length: 0, angle: 0, percent: 0, hint: None };
    const LENGTH: Type = Type { length: 1, ..Type::NUMBER };
    const ANGLE: Type = Type { length: 0, angle: 1, ..Type::NUMBER };
    const PERCENT: Type = Type { percent: 1, ..Type::NUMBER };

    fn power(&mut self, base: Base) -> &mut i8 {
        match base {
            Base::Length => &mut self.length,
            Base::Angle => &mut self.angle,
        }
    }

    fn powers(self) -> (i8, i8, i8) {
        (self.length, self.angle, self.percent)
    }

    fn apply_hint(&mut self, hint: Base) {
        let percent = std::mem::take(&mut self.percent);
        *self.power(hint) += percent;
        self.hint = Some(hint);
    }

    /// Gives both types the same percent hint, if they can have one.
    fn unify_hints(a: &mut Type, b: &mut Type) -> Option<()> {
        match (a.hint, b.hint) {
            (Some(x), Some(y)) if x != y => return None,
            (Some(x), None) => b.apply_hint(x),
            (None, Some(y)) => a.apply_hint(y),
            _ => {}
        }
        Some(())
    }

    /// The type of a sum. https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-add-two-types
    fn add(mut a: Type, mut b: Type) -> Option<Type> {
        Type::unify_hints(&mut a, &mut b)?;
        if a.powers() == b.powers() {
            return Some(Type { hint: a.hint.or(b.hint), ..a });
        }
        let has_other = |t: Type| t.length != 0 || t.angle != 0;
        if (a.percent != 0 || b.percent != 0) && (has_other(a) || has_other(b)) {
            for hint in [Base::Length, Base::Angle] {
                let (mut a, mut b) = (a, b);
                a.apply_hint(hint);
                b.apply_hint(hint);
                if a.powers() == b.powers() {
                    return Some(a);
                }
            }
        }
        None
    }

    /// The type of a product. https://drafts.css-houdini.org/css-typed-om-1/#cssnumericvalue-multiply-two-types
    fn multiply(mut a: Type, mut b: Type) -> Option<Type> {
        Type::unify_hints(&mut a, &mut b)?;
        Some(Type {
            length: a.length.checked_add(b.length)?,
            angle: a.angle.checked_add(b.angle)?,
            percent: a.percent.checked_add(b.percent)?,
            hint: a.hint.or(b.hint),
        })
    }

    fn invert(self) -> Type {
        Type { length: -self.length, angle: -self.angle, percent: -self.percent, hint: self.hint }
    }

    fn is_number(self) -> bool {
        self.powers() == (0, 0, 0)
    }

    /// A number that keeps the percent hint of the calculation it comes from ("made consistent").
    fn number_like(self) -> Type {
        Type { hint: self.hint, ..Type::NUMBER }
    }
}

/// The type of value that a math function must have where it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    /// `<length>`
    Length,
    /// `<length-percentage>`: the percentages refer to a length.
    LengthPercentage,
    /// `<number>` (and `<integer>`, which is rounded)
    Number,
    /// `<number> | <percentage>`, where percentages are fractions of 1 (`opacity`).
    NumberOrPercentage,
    /// `<angle>`
    Angle,
    /// `<angle-percentage>`: the percentages refer to an angle.
    AnglePercentage,
}

impl Expected {
    /// The type of a percentage in the calculation.
    fn percentage_type(self) -> Type {
        match self {
            Expected::LengthPercentage => {
                let mut t = Type::PERCENT;
                t.apply_hint(Base::Length);
                t
            }
            Expected::AnglePercentage => {
                let mut t = Type::PERCENT;
                t.apply_hint(Base::Angle);
                t
            }
            _ => Type::PERCENT,
        }
    }

    fn matches(self, t: Type) -> bool {
        match self {
            Expected::Length => t.powers() == (1, 0, 0) && t.hint.is_none(),
            Expected::LengthPercentage => t.powers() == (1, 0, 0) || (t.powers() == (0, 0, 1) && t.hint.is_none()),
            Expected::Number => t.is_number() && t.hint.is_none(),
            Expected::NumberOrPercentage => (t.is_number() || t.powers() == (0, 0, 1)) && t.hint.is_none(),
            Expected::Angle => t.powers() == (0, 1, 0) && t.hint.is_none(),
            Expected::AnglePercentage => t.powers() == (0, 1, 0) || (t.powers() == (0, 0, 1) && t.hint.is_none()),
        }
    }
}

// --- parsing ---

/// Whether a function is a math function.
pub fn is_math_function(name: &str) -> bool {
    function_kind(name).is_some()
}

enum FunctionKind {
    Calc,
    Math(MathFunction),
}

fn function_kind(name: &str) -> Option<FunctionKind> {
    use MathFunction::*;
    Some(FunctionKind::Math(match_ignore_ascii_case! { name,
        "calc" => return Some(FunctionKind::Calc),
        "min" => Min,
        "max" => Max,
        "clamp" => Clamp,
        "round" => Round(Rounding::Nearest),
        "mod" => Mod,
        "rem" => Rem,
        "sin" => Sin,
        "cos" => Cos,
        "tan" => Tan,
        "asin" => Asin,
        "acos" => Acos,
        "atan" => Atan,
        "atan2" => Atan2,
        "pow" => Pow,
        "sqrt" => Sqrt,
        "hypot" => Hypot,
        "log" => Log,
        "exp" => Exp,
        "abs" => Abs,
        "sign" => Sign,
        _ => return None,
    }))
}

type ParseResult<T, E> = Result<T, ParseError<E>>;

/// The error of a calculation that is not valid (of the wrong type, say).
fn invalid<E>() -> ParseError<E> {
    ParseError::unexpected_token()
}

/// Parses a math function of the expected type, with the given range.
pub fn parse<'i>(input: &mut Parser<'i>, expected: Expected, range: Range) -> ParseResult<Calc, StyleParseError> {
    input.try_parse(|input| {
        let name = input.expect_function()?.clone();
        let Some(kind) = function_kind(&name) else {
            return Err(invalid());
        };
        let (node, t) = input.parse_nested_block(|input| parse_function(kind, input, expected))?;
        if !expected.matches(t) {
            return Err(invalid());
        }
        Ok(Calc::new(node, range))
    })
}

/// Parses a math function that resolves to a number when it is parsed (one that has no relative
/// lengths): a `<number>`, or the fraction of a percentage for [`Expected::NumberOrPercentage`].
/// The result is censored but not clamped.
pub fn parse_number<'i>(input: &mut Parser<'i>, expected: Expected) -> ParseResult<f32, StyleParseError> {
    input.try_parse(|input| {
        let calc = parse(input, expected, Range::All)?;
        if calc.0.node.has_relative_length() {
            // TODO: a number such as `calc(1em / 1px)` is only known when the font size is
            return Err(invalid());
        }
        // a percentage of `opacity` is a fraction of 1
        Ok(calc.resolve(1.0))
    })
}

/// Parses a math function that is an `<integer>`: it is rounded to the nearest integer (halves
/// toward positive infinity).
pub fn parse_integer<'i>(input: &mut Parser<'i>) -> ParseResult<i32, StyleParseError> {
    parse_number(input, Expected::Number).map(|value| (value + 0.5).floor().clamp(i32::MIN as f32, i32::MAX as f32) as i32)
}

/// Parses a math function that is an `<angle>`, in degrees.
pub fn parse_angle<'i>(input: &mut Parser<'i>) -> ParseResult<f32, StyleParseError> {
    parse_number(input, Expected::Angle).map(f32::to_degrees)
}

/// Parses a math function that is an `<angle-percentage>`, in degrees, where 100% is `full`
/// degrees (angles have no relative units, so the percentages are resolved now).
pub fn parse_angle_percentage<'i>(input: &mut Parser<'i>, full: f32) -> ParseResult<f32, StyleParseError> {
    parse(input, Expected::AnglePercentage, Range::All).map(|calc| calc.resolve(full.to_radians()).to_degrees())
}

fn parse_function<'i, E>(kind: FunctionKind, input: &mut Parser<'i>, expected: Expected) -> ParseResult<(Node, Type), E> {
    use MathFunction::*;
    let function = match kind {
        FunctionKind::Calc => {
            let result = parse_sum(input, expected)?;
            input.expect_exhausted()?;
            return Ok(result);
        }
        FunctionKind::Math(function) => function,
    };
    // arguments that must all have the same type, which is that of the result
    let same_type = |args: &[(Node, Type)]| -> Option<Type> {
        let mut iter = args.iter().map(|(_, t)| *t);
        let first = iter.next()?;
        iter.try_fold(first, Type::add)
    };
    let numbers = |args: &[(Node, Type)]| -> Option<Type> {
        let t = same_type(args)?;
        t.is_number().then_some(t)
    };
    let (function, args, t) = match function {
        Min | Max | Hypot => {
            let args = input.parse_comma_separated(|input| parse_sum(input, expected))?;
            let t = same_type(&args).ok_or_else(invalid::<E>)?;
            (function, args, t)
        }
        Clamp => {
            let mut args = Vec::with_capacity(3);
            let mut types = Vec::with_capacity(3);
            for (i, bound) in [f32::NEG_INFINITY, f32::NAN, f32::INFINITY].into_iter().enumerate() {
                if i > 0 {
                    input.expect_comma()?;
                }
                // the minimum and the maximum may be `none`
                if !bound.is_nan() && input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
                    args.push((Node::Number(Bits(bound)), Type::NUMBER));
                    continue;
                }
                let arg = parse_sum(input, expected)?;
                types.push(arg.1);
                args.push(arg);
            }
            let mut iter = types.into_iter();
            let first = iter.next().ok_or_else(invalid::<E>)?;
            let t = iter.try_fold(first, Type::add).ok_or_else(invalid::<E>)?;
            (function, args, t)
        }
        Round(_) => {
            let strategy = input
                .try_parse(|input| {
                    let ident = input.expect_ident()?.clone();
                    let strategy = match_ignore_ascii_case! { &ident,
                        "nearest" => Rounding::Nearest,
                        "up" => Rounding::Up,
                        "down" => Rounding::Down,
                        "to-zero" => Rounding::ToZero,
                        _ => return Err(input.new_error_for_next_token::<E>()),
                    };
                    input.expect_comma()?;
                    Ok(strategy)
                })
                .unwrap_or(Rounding::Nearest);
            let mut args = vec![parse_sum(input, expected)?];
            if input.try_parse(|input| input.expect_comma()).is_ok() {
                args.push(parse_sum(input, expected)?);
            } else if args[0].1.is_number() {
                // the step of a number may be left out: it is 1
                args.push((Node::Number(Bits(1.0)), Type::NUMBER));
            } else {
                return Err(invalid());
            }
            let t = same_type(&args).ok_or_else(invalid::<E>)?;
            (Round(strategy), args, t)
        }
        Mod | Rem | Atan2 => {
            let a = parse_sum(input, expected)?;
            input.expect_comma()?;
            let b = parse_sum(input, expected)?;
            let args = vec![a, b];
            let t = same_type(&args).ok_or_else(invalid::<E>)?;
            let t = if function == Atan2 { Type { hint: t.hint, ..Type::ANGLE } } else { t };
            (function, args, t)
        }
        Sin | Cos | Tan => {
            let arg = parse_sum(input, expected)?;
            if !(arg.1.is_number() || arg.1.powers() == (0, 1, 0)) {
                return Err(invalid());
            }
            let t = arg.1.number_like();
            (function, vec![arg], t)
        }
        Asin | Acos | Atan => {
            let args = vec![parse_sum(input, expected)?];
            let t = numbers(&args).ok_or_else(invalid::<E>)?;
            (function, args, Type { hint: t.hint, ..Type::ANGLE })
        }
        Pow => {
            let a = parse_sum(input, expected)?;
            input.expect_comma()?;
            let b = parse_sum(input, expected)?;
            let args = vec![a, b];
            let t = numbers(&args).ok_or_else(invalid::<E>)?;
            (function, args, t)
        }
        Sqrt | Exp => {
            let args = vec![parse_sum(input, expected)?];
            let t = numbers(&args).ok_or_else(invalid::<E>)?;
            (function, args, t)
        }
        Log => {
            let mut args = vec![parse_sum(input, expected)?];
            if input.try_parse(|input| input.expect_comma()).is_ok() {
                args.push(parse_sum(input, expected)?);
            }
            let t = numbers(&args).ok_or_else(invalid::<E>)?;
            (function, args, t)
        }
        Abs => {
            let arg = parse_sum(input, expected)?;
            let t = arg.1;
            (function, vec![arg], t)
        }
        Sign => {
            let arg = parse_sum(input, expected)?;
            let t = arg.1.number_like();
            (function, vec![arg], t)
        }
    };
    input.expect_exhausted()?;
    let children = args.into_iter().map(|(node, _)| node).collect();
    Ok((Node::Function(function, children), t))
}

/// `<calc-sum> = <calc-product> [ [ '+' | '-' ] <calc-product> ]*`, where the operators must
/// have white space on both sides.
fn parse_sum<'i, E>(input: &mut Parser<'i>, expected: Expected) -> ParseResult<(Node, Type), E> {
    let (first, mut t) = parse_product(input, expected)?;
    let mut terms = vec![first];
    loop {
        let state = input.state();
        if !matches!(input.next_including_whitespace(), Ok(Token::WhiteSpace(_))) {
            input.reset(&state);
            break;
        }
        let negate = match input.next_including_whitespace() {
            Ok(Token::Delim('+')) => false,
            Ok(Token::Delim('-')) => true,
            _ => {
                // white space before a comma or the end
                input.reset(&state);
                break;
            }
        };
        if !matches!(input.next_including_whitespace(), Ok(Token::WhiteSpace(_))) {
            return Err(input.new_error_for_next_token());
        }
        let (term, term_type) = parse_product(input, expected)?;
        t = Type::add(t, term_type).ok_or_else(|| invalid())?;
        terms.push(if negate { Node::Negate(Box::new(term)) } else { term });
    }
    Ok(if terms.len() == 1 { (terms.pop().unwrap(), t) } else { (Node::Sum(terms.into()), t) })
}

/// `<calc-product> = <calc-value> [ [ '*' | '/' ] <calc-value> ]*`
fn parse_product<'i, E>(input: &mut Parser<'i>, expected: Expected) -> ParseResult<(Node, Type), E> {
    let (first, mut t) = parse_value(input, expected)?;
    let mut factors = vec![first];
    loop {
        let divide = match input.try_parse(|input| match input.next() {
            Ok(Token::Delim('*')) => Ok(false),
            Ok(Token::Delim('/')) => Ok(true),
            _ => Err(()),
        }) {
            Ok(divide) => divide,
            Err(()) => break,
        };
        let (factor, factor_type) = parse_value(input, expected)?;
        let (factor, factor_type) =
            if divide { (Node::Invert(Box::new(factor)), factor_type.invert()) } else { (factor, factor_type) };
        t = Type::multiply(t, factor_type).ok_or_else(|| invalid())?;
        factors.push(factor);
    }
    Ok(if factors.len() == 1 { (factors.pop().unwrap(), t) } else { (Node::Product(factors.into()), t) })
}

/// `<calc-value> = <number> | <dimension> | <percentage> | <calc-keyword> | ( <calc-sum> )`, or a
/// math function.
fn parse_value<'i, E>(input: &mut Parser<'i>, expected: Expected) -> ParseResult<(Node, Type), E> {
    let token = input.next()?.clone();
    match token {
        Token::Number { value, .. } => Ok((Node::Number(Bits(value)), Type::NUMBER)),
        Token::Percentage { unit_value, .. } => Ok((Node::Percentage(Bits(unit_value)), expected.percentage_type())),
        Token::Dimension { value, ref unit, .. } => {
            if let Ok(unit) = unit.parse::<Unit>() {
                return Ok((Node::Length(Bits(value), unit), Type::LENGTH));
            }
            let radians = match_ignore_ascii_case! { unit,
                "deg" => value.to_radians(),
                "grad" => (value * 0.9).to_radians(),
                "rad" => value,
                "turn" => value * std::f32::consts::TAU,
                _ => return Err(invalid()),
            };
            Ok((Node::Angle(Bits(radians)), Type::ANGLE))
        }
        Token::Ident(ref ident) => {
            let value = match_ignore_ascii_case! { ident,
                "e" => std::f32::consts::E,
                "pi" => std::f32::consts::PI,
                "infinity" => f32::INFINITY,
                "-infinity" => f32::NEG_INFINITY,
                "nan" => f32::NAN,
                _ => return Err(invalid()),
            };
            Ok((Node::Number(Bits(value)), Type::NUMBER))
        }
        Token::ParenthesisBlock => input.parse_nested_block(|input| {
            let result = parse_sum(input, expected)?;
            input.expect_exhausted()?;
            Ok(result)
        }),
        Token::Function(ref name) => match function_kind(name) {
            Some(kind) => input.parse_nested_block(|input| parse_function(kind, input, expected)),
            None => Err(invalid()),
        },
        _ => Err(invalid()),
    }
}

// --- serialization, for debugging ---

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn list(f: &mut fmt::Formatter<'_>, children: &[Node], separator: &str) -> fmt::Result {
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    f.write_str(separator)?;
                }
                write!(f, "{}", child)?;
            }
            Ok(())
        }
        match self {
            Node::Number(Bits(value)) => write!(f, "{}", value),
            Node::Percentage(Bits(fraction)) => write!(f, "{}%", fraction * 100.0),
            Node::Length(Bits(value), unit) => write!(f, "{}{}", value, format!("{:?}", unit).to_ascii_lowercase()),
            Node::Angle(Bits(radians)) => write!(f, "{}deg", radians.to_degrees()),
            Node::Sum(children) => {
                f.write_str("(")?;
                list(f, children, " + ")?;
                f.write_str(")")
            }
            Node::Product(children) => list(f, children, " * "),
            Node::Negate(child) => write!(f, "-1 * ({})", child),
            Node::Invert(child) => write!(f, "1 / ({})", child),
            Node::Function(function, children) => {
                let name = match function {
                    MathFunction::Round(Rounding::Nearest) => "round(nearest, ".to_owned(),
                    MathFunction::Round(Rounding::Up) => "round(up, ".to_owned(),
                    MathFunction::Round(Rounding::Down) => "round(down, ".to_owned(),
                    MathFunction::Round(Rounding::ToZero) => "round(to-zero, ".to_owned(),
                    other => format!("{}(", format!("{:?}", other).to_ascii_lowercase()),
                };
                f.write_str(&name)?;
                list(f, children, ", ")?;
                f.write_str(")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(css: &str, expected: Expected) -> Option<Calc> {
        Parser::new(css)
            .parse_entirely(|input| parse(input, expected, Range::All))
            .ok()
    }

    fn sizes() -> FontSizes {
        FontSizes { font: PixelLength(16.0), root: PixelLength(10.0), viewport: (PixelLength(800.0), PixelLength(1000.0)), ..FontSizes::default() }
    }

    fn px(css: &str) -> f32 {
        parse_str(css, Expected::Length).unwrap_or_else(|| panic!("{} is not a length", css)).to_px(sizes()).0
    }

    fn number(css: &str) -> f32 {
        Parser::new(css)
            .parse_entirely(|input| parse_number(input, Expected::Number))
            .unwrap_or_else(|_| panic!("{} is not a number", css))
    }

    fn lp(css: &str) -> LengthPercentage {
        parse_str(css, Expected::LengthPercentage).unwrap_or_else(|| panic!("{} is not a length-percentage", css)).to_length_percentage(sizes())
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3 || (a.is_nan() && b.is_nan())
    }

    #[test]
    fn arithmetic() {
        assert_eq!(px("calc(1px + 2px)"), 3.0);
        assert_eq!(px("calc(10px - 2px * 3)"), 4.0);
        assert_eq!(px("calc((10px - 2px) * 3)"), 24.0);
        assert_eq!(px("calc(1em + 1rem)"), 26.0);
        assert_eq!(px("calc(10vw / 2)"), 40.0);
        assert_eq!(px("calc(1in - 90px)"), 6.0);
        assert_eq!(px("calc(2 * 3px)"), 6.0);
        assert_eq!(px("calc(-1 * (2px))"), -2.0);
        assert_eq!(px("CALC(1PX)"), 1.0);
        // typed arithmetic: lengths may be multiplied and divided
        assert_eq!(px("calc(2px * 3px / 1px)"), 6.0);
        assert_eq!(px("calc(1em * 1em / 4px)"), 64.0);
        // nested functions
        assert_eq!(px("calc(min(1px, 2px) + max(3px, 4px))"), 5.0);
    }

    #[test]
    fn syntax() {
        // `+` and `-` need white space on both sides, `*` and `/` do not
        assert!(parse_str("calc(1px+2px)", Expected::Length).is_none());
        assert!(parse_str("calc(1px -2px)", Expected::Length).is_none());
        assert!(parse_str("calc(1px- 2px)", Expected::Length).is_none());
        assert!(parse_str("calc(1px*2)", Expected::Length).is_some());
        assert!(parse_str("calc( 1px  +  2px )", Expected::Length).is_some());
        assert!(parse_str("calc()", Expected::Length).is_none());
        assert!(parse_str("calc(1px 2px)", Expected::Length).is_none());
        assert!(parse_str("calc(1px, 2px)", Expected::Length).is_none());
        assert!(parse_str("min()", Expected::Length).is_none());
        assert!(parse_str("foo(1px)", Expected::Length).is_none());
    }

    #[test]
    fn types() {
        // a length plus a number is not anything
        assert!(parse_str("calc(1px + 2)", Expected::Length).is_none());
        assert!(parse_str("calc(1px * 2px)", Expected::Length).is_none());
        assert!(parse_str("calc(2)", Expected::Length).is_none());
        assert!(parse_str("calc(1px / 1px)", Expected::Number).is_some());
        // a percentage is not a `<length>`
        assert!(parse_str("calc(1px + 10%)", Expected::Length).is_none());
        assert!(parse_str("calc(10%)", Expected::Length).is_none());
        assert!(parse_str("calc(1px + 10%)", Expected::LengthPercentage).is_some());
        assert!(parse_str("calc(10%)", Expected::LengthPercentage).is_some());
        assert!(parse_str("calc(10% * 10%)", Expected::LengthPercentage).is_none());
        // percentages that cancel are a number
        assert_eq!(px("calc(2px * (50% / 25%))"), 4.0);
        assert!(parse_str("calc(1deg + 1px)", Expected::Length).is_none());
        assert!(parse_str("calc(1s)", Expected::Length).is_none());
        assert!(parse_str("calc(1px)", Expected::Number).is_none());
        assert!(parse_str("calc(50%)", Expected::NumberOrPercentage).is_some());
        assert!(parse_str("calc(50% + 1)", Expected::NumberOrPercentage).is_none());
    }

    #[test]
    fn length_percentage() {
        use LengthPercentage as LP;
        assert_eq!(lp("calc(10px + 1em)"), LP::PixelLength(PixelLength(26.0)));
        assert_eq!(lp("calc(50%)"), LP::Percentage(Percentage(0.5)));
        assert_eq!(lp("calc(100% - 2em)"), LP::Both { length: PixelLength(-32.0), percentage: Percentage(1.0) });
        assert_eq!(lp("calc((100% - 10px) / 2)"), LP::Both { length: PixelLength(-5.0), percentage: Percentage(0.5) });
        assert_eq!(lp("calc(2 * (10% + 1px) - 1px)"), LP::Both { length: PixelLength(1.0), percentage: Percentage(0.2) });
        let LP::Calc(min) = lp("min(100%, 40em)") else { panic!("min() of a percentage is a calculation") };
        assert_eq!(min.resolve(1000.0), 640.0);
        assert_eq!(min.resolve(300.0), 300.0);
        let LP::Calc(clamped) = lp("clamp(10px, 50%, 100px)") else { panic!() };
        assert_eq!(clamped.resolve(100.0), 50.0);
        assert_eq!(clamped.resolve(10.0), 10.0);
        assert_eq!(clamped.resolve(1000.0), 100.0);
        // the same calculation of the same sizes is the same tree
        let (LP::Calc(a), LP::Calc(b)) = (lp("min(100%, 40em)"), lp("min(100%, 40em)")) else { panic!() };
        assert_eq!(a.as_ptr(), b.as_ptr());
    }

    #[test]
    fn non_negative_range() {
        let calc = Parser::new("calc(50% - 100px)")
            .parse_entirely(|input| parse(input, Expected::LengthPercentage, Range::NonNegative))
            .unwrap();
        let LengthPercentage::Calc(computed) = calc.to_length_percentage(sizes()) else { panic!("only the sum is clamped") };
        assert_eq!(computed.resolve(100.0), 0.0);
        assert_eq!(computed.resolve(400.0), 100.0);
        let negative = Parser::new("calc(-5px)")
            .parse_entirely(|input| parse(input, Expected::Length, Range::NonNegative))
            .unwrap();
        assert_eq!(negative.to_px(sizes()).0, 0.0);
    }

    #[test]
    fn functions() {
        assert_eq!(number("min(3, 1, 2)"), 1.0);
        assert_eq!(number("max(3, 1, 2)"), 3.0);
        assert_eq!(number("clamp(1, 5, 3)"), 3.0);
        assert_eq!(number("clamp(1, 0, 3)"), 1.0);
        // the minimum wins over the maximum
        assert_eq!(number("clamp(3, 2, 1)"), 3.0);
        assert_eq!(number("clamp(none, 5, 3)"), 3.0);
        assert_eq!(number("clamp(4, 2, none)"), 4.0);
        assert_eq!(number("round(2.5)"), 3.0);
        assert_eq!(number("round(-2.5)"), -2.0);
        assert_eq!(number("round(up, 2.1, 1)"), 3.0);
        assert_eq!(number("round(down, 2.9, 1)"), 2.0);
        assert_eq!(number("round(to-zero, -2.9, 1)"), -2.0);
        assert_eq!(number("round(17, 5)"), 15.0);
        assert_eq!(px("round(17px, 5px)"), 15.0);
        assert_eq!(px("round(up, 1.5em, 5px)"), 25.0);
        assert_eq!(number("mod(18, 5)"), 3.0);
        assert_eq!(number("mod(-18, 5)"), 2.0);
        assert_eq!(number("rem(-18, 5)"), -3.0);
        assert_eq!(number("mod(18, -5)"), -2.0);
        assert_eq!(px("mod(18px, 5px)"), 3.0);
        assert!(approx(number("sin(90deg)"), 1.0));
        assert!(approx(number("cos(pi)"), -1.0));
        assert!(approx(number("tan(45deg)"), 1.0));
        assert!(approx(number("sin(0.25turn)"), 1.0));
        assert!(approx(number("calc(asin(1) / 1deg)"), 90.0));
        assert!(approx(number("calc(acos(-1) / 1rad)"), std::f32::consts::PI));
        assert!(approx(number("calc(atan2(1px, 1px) / 1deg)"), 45.0));
        assert!(approx(number("calc(atan2(-1, 0) / 1deg)"), -90.0));
        assert_eq!(number("pow(2, 10)"), 1024.0);
        assert_eq!(number("sqrt(16)"), 4.0);
        assert_eq!(px("hypot(3px, 4px)"), 5.0);
        assert!(approx(number("log(e)"), 1.0));
        assert!(approx(number("log(8, 2)"), 3.0));
        assert!(approx(number("exp(0)"), 1.0));
        assert_eq!(px("abs(-3px)"), 3.0);
        assert_eq!(number("calc(sign(-3px) * 2)"), -2.0);
        assert_eq!(number("calc(-3px / 1px)"), -3.0);
        assert_eq!(px("calc(10px * sign(-3em))"), -10.0);
        // arguments of the wrong type
        assert!(parse_str("pow(2px, 2)", Expected::Length).is_none());
        assert!(parse_str("sqrt(4px)", Expected::Length).is_none());
        assert!(parse_str("sin(1px)", Expected::Number).is_none());
        assert!(parse_str("min(1px, 2)", Expected::Length).is_none());
        assert!(parse_str("round(1px)", Expected::Length).is_none());
        assert!(parse_str("clamp(1px, 2px)", Expected::Length).is_none());
    }

    #[test]
    fn infinity_and_nan() {
        // a top-level `NaN` is 0 and infinity is the largest value
        assert_eq!(px("calc(NaN * 1px)"), 0.0);
        assert_eq!(px("calc(infinity * 1px)"), LIMIT);
        assert_eq!(px("calc(-infinity * 1px)"), -LIMIT);
        assert_eq!(px("calc(1px / 0)"), LIMIT);
        assert_eq!(px("calc(-1px / 0)"), -LIMIT);
        // `NaN` in min() is `NaN`
        assert_eq!(px("min(1px, NaN * 1px)"), 0.0);
        assert_eq!(number("calc(1 / -0)"), -LIMIT);
        assert_eq!(number("round(5, 0)"), 0.0);
        assert_eq!(number("mod(5, infinity)"), 5.0);
        assert_eq!(number("mod(-5, infinity)"), 0.0);
        assert_eq!(number("rem(-5, infinity)"), -5.0);
        // rounding to an infinite step keeps the sign of zero
        assert_eq!(number("calc(1 / round(up, 0.5, infinity))"), 0.0);
        assert_eq!(number("calc(1 / round(up, -0.5, infinity))"), -LIMIT);
        assert_eq!(number("calc(1 / round(down, 0.5, infinity))"), LIMIT);
        assert_eq!(number("calc(1 / round(down, -0.5, infinity))"), 0.0);
        assert_eq!(number("calc(1 / round(-0.4))"), -LIMIT);
        assert_eq!(number("calc(1 / sign(-0))"), -LIMIT);
        assert_eq!(number("pow(infinity, -1)"), 0.0);
        assert_eq!(number("calc(e - e)"), 0.0);
        assert_eq!(number("calc(Infinity - INFINITY)"), 0.0);
    }

    #[test]
    fn integers_and_angles() {
        let integer = |css: &str| {
            Parser::new(css).parse_entirely(|input| parse_integer(input)).unwrap()
        };
        assert_eq!(integer("calc(2.5)"), 3);
        assert_eq!(integer("calc(-2.5)"), -2);
        assert_eq!(integer("calc(7 / 2)"), 4);
        let angle = |css: &str| {
            Parser::new(css).parse_entirely(|input| parse_angle(input)).unwrap()
        };
        assert!(approx(angle("calc(0.25turn + 10deg)"), 100.0));
        assert!(approx(angle("calc(pi * 1rad)"), 180.0));
        assert!(approx(angle("atan(1)"), 45.0));
        let angle_percentage = |css: &str| {
            Parser::new(css).parse_entirely(|input| parse_angle_percentage(input, 360.0)).unwrap()
        };
        assert!(approx(angle_percentage("calc(90deg + 25%)"), 180.0));
        assert!(approx(angle_percentage("max(0%, 25%)"), 90.0));
        assert!(parse_str("calc(1px + 25%)", Expected::AnglePercentage).is_none());
    }
}
