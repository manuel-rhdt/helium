pub mod specified {
    use cssparser::{CowRcStr, ParseError, Parser, Token};
    use from_css_decl_derive::FromCssDecl;
    use std::str::FromStr;
    pub trait FromCssDecl<'i>: Sized {
        type Error: 'i;
        fn from_css_decl(
            name: CowRcStr<'i>,
            input: &mut Parser<'i, '_>,
        ) -> Result<Self, Self::Error>;
    }
    #[rustc_copy_clone_marker]
    pub enum PropertyDeclaration {
        Width(QuantityOrPercentage),
        Height(QuantityOrPercentage),
        Display(Display),
        FontSize(FontSize),
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::fmt::Debug for PropertyDeclaration {
        fn fmt(&self, f: &mut $crate::fmt::Formatter) -> $crate::fmt::Result {
            match (&*self,) {
                (&PropertyDeclaration::Width(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("Width");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
                (&PropertyDeclaration::Height(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("Height");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
                (&PropertyDeclaration::Display(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("Display");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
                (&PropertyDeclaration::FontSize(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("FontSize");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::marker::Copy for PropertyDeclaration {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::clone::Clone for PropertyDeclaration {
        #[inline]
        fn clone(&self) -> PropertyDeclaration {
            {
                let _: $crate::clone::AssertParamIsClone<QuantityOrPercentage>;
                let _: $crate::clone::AssertParamIsClone<QuantityOrPercentage>;
                let _: $crate::clone::AssertParamIsClone<Display>;
                let _: $crate::clone::AssertParamIsClone<FontSize>;
                *self
            }
        }
    }
    impl<'i> FromCssDecl<'i> for PropertyDeclaration {
        type Error = cssparser::ParseError<'i, ()>;
        fn from_css_decl(
            name: cssparser::CowRcStr<'i>,
            input: &mut cssparser::Parser<'i, '_>,
        ) -> Result<PropertyDeclaration, Self::Error> {
            if name.eq_ignore_ascii_case("width") {
                return <QuantityOrPercentage as FromCssDecl>::from_css_decl(name, input)
                    .map(PropertyDeclaration::Width);
            }
            if name.eq_ignore_ascii_case("height") {
                return <QuantityOrPercentage as FromCssDecl>::from_css_decl(name, input)
                    .map(PropertyDeclaration::Height);
            }
            if name.eq_ignore_ascii_case("display") {
                return <Display as FromCssDecl>::from_css_decl(name, input)
                    .map(PropertyDeclaration::Display);
            }
            if name.eq_ignore_ascii_case("font-size") {
                return <FontSize as FromCssDecl>::from_css_decl(name, input)
                    .map(PropertyDeclaration::FontSize);
            }
            Err(input.new_custom_error(()))
        }
    }
    #[structural_match]
    #[rustc_copy_clone_marker]
    pub enum Display {
        None,
        Block,
        Inline,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::fmt::Debug for Display {
        fn fmt(&self, f: &mut $crate::fmt::Formatter) -> $crate::fmt::Result {
            match (&*self,) {
                (&Display::None,) => {
                    let mut debug_trait_builder = f.debug_tuple("None");
                    debug_trait_builder.finish()
                }
                (&Display::Block,) => {
                    let mut debug_trait_builder = f.debug_tuple("Block");
                    debug_trait_builder.finish()
                }
                (&Display::Inline,) => {
                    let mut debug_trait_builder = f.debug_tuple("Inline");
                    debug_trait_builder.finish()
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::marker::Copy for Display {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::clone::Clone for Display {
        #[inline]
        fn clone(&self) -> Display {
            {
                *self
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::cmp::PartialEq for Display {
        #[inline]
        fn eq(&self, other: &Display) -> bool {
            {
                let __self_vi = unsafe { $crate::intrinsics::discriminant_value(&*self) } as isize;
                let __arg_1_vi =
                    unsafe { $crate::intrinsics::discriminant_value(&*other) } as isize;
                if true && __self_vi == __arg_1_vi {
                    match (&*self, &*other) {
                        _ => true,
                    }
                } else {
                    false
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::cmp::Eq for Display {
        #[inline]
        #[doc(hidden)]
        fn assert_receiver_is_total_eq(&self) -> () {
            {}
        }
    }
    impl Display {
        pub fn parse<'i>(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
            match input.next()? {
                Token::Ident(ident) => match ident.parse() {
                    Ok(display) => Ok(display),
                    Err(_) => {
                        let ident = ident.clone();
                        Err(input.new_unexpected_token_error(Token::Ident(ident)))
                    }
                },
                unexpected => {
                    let unexpected = unexpected.clone();
                    Err(input.new_basic_unexpected_token_error(unexpected).into())
                }
            }
        }
    }
    impl<'i> FromCssDecl<'i> for Display {
        type Error = ParseError<'i, ()>;
        fn from_css_decl(
            _name: CowRcStr<'i>,
            input: &mut Parser<'i, '_>,
        ) -> Result<Self, ParseError<'i, ()>> {
            Self::parse(input)
        }
    }
    impl FromStr for Display {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "none" => Ok(Display::None),
                "block" => Ok(Display::Block),
                "inline" => Ok(Display::Inline),
                _ => Err(()),
            }
        }
    }
    #[structural_match]
    #[rustc_copy_clone_marker]
    pub enum Unit {
        Px,
        Cm,
        Pt,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::fmt::Debug for Unit {
        fn fmt(&self, f: &mut $crate::fmt::Formatter) -> $crate::fmt::Result {
            match (&*self,) {
                (&Unit::Px,) => {
                    let mut debug_trait_builder = f.debug_tuple("Px");
                    debug_trait_builder.finish()
                }
                (&Unit::Cm,) => {
                    let mut debug_trait_builder = f.debug_tuple("Cm");
                    debug_trait_builder.finish()
                }
                (&Unit::Pt,) => {
                    let mut debug_trait_builder = f.debug_tuple("Pt");
                    debug_trait_builder.finish()
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::marker::Copy for Unit {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::clone::Clone for Unit {
        #[inline]
        fn clone(&self) -> Unit {
            {
                *self
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::cmp::PartialEq for Unit {
        #[inline]
        fn eq(&self, other: &Unit) -> bool {
            {
                let __self_vi = unsafe { $crate::intrinsics::discriminant_value(&*self) } as isize;
                let __arg_1_vi =
                    unsafe { $crate::intrinsics::discriminant_value(&*other) } as isize;
                if true && __self_vi == __arg_1_vi {
                    match (&*self, &*other) {
                        _ => true,
                    }
                } else {
                    false
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::cmp::Eq for Unit {
        #[inline]
        #[doc(hidden)]
        fn assert_receiver_is_total_eq(&self) -> () {
            {}
        }
    }
    impl FromStr for Unit {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            match s {
                "px" => Ok(Unit::Px),
                "cm" => Ok(Unit::Cm),
                "pt" => Ok(Unit::Pt),
                _ => Err(()),
            }
        }
    }
    #[rustc_copy_clone_marker]
    pub enum QuantityOrPercentage {
        Quantity { value: super::CssFloat, unit: Unit },
        Percentage(super::Percentage),
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::fmt::Debug for QuantityOrPercentage {
        fn fmt(&self, f: &mut $crate::fmt::Formatter) -> $crate::fmt::Result {
            match (&*self,) {
                (&QuantityOrPercentage::Quantity {
                    value: ref __self_0,
                    unit: ref __self_1,
                },) => {
                    let mut debug_trait_builder = f.debug_struct("Quantity");
                    let _ = debug_trait_builder.field("value", &&(*__self_0));
                    let _ = debug_trait_builder.field("unit", &&(*__self_1));
                    debug_trait_builder.finish()
                }
                (&QuantityOrPercentage::Percentage(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("Percentage");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::marker::Copy for QuantityOrPercentage {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::clone::Clone for QuantityOrPercentage {
        #[inline]
        fn clone(&self) -> QuantityOrPercentage {
            {
                let _: $crate::clone::AssertParamIsClone<super::CssFloat>;
                let _: $crate::clone::AssertParamIsClone<Unit>;
                let _: $crate::clone::AssertParamIsClone<super::Percentage>;
                *self
            }
        }
    }
    impl QuantityOrPercentage {
        pub fn parse<'a, 'b>(token: &'a Token<'b>) -> Result<Self, Token<'b>> {
            let result = match token {
                Token::Dimension { value, unit, .. } => QuantityOrPercentage::Quantity {
                    value: *value,
                    unit: unit.parse().unwrap(),
                },
                Token::Percentage { unit_value, .. } => {
                    QuantityOrPercentage::Percentage(super::Percentage(*unit_value))
                }
                unexpected => return Err(unexpected.clone()),
            };
            Ok(result)
        }
    }
    impl<'i> FromCssDecl<'i> for QuantityOrPercentage {
        type Error = ParseError<'i, ()>;
        fn from_css_decl(
            _name: CowRcStr<'i>,
            input: &mut Parser<'i, '_>,
        ) -> Result<Self, ParseError<'i, ()>> {
            QuantityOrPercentage::parse(input.next()?).map_err(|unexpected_token| {
                input
                    .new_basic_unexpected_token_error(unexpected_token)
                    .into()
            })
        }
    }
    #[rustc_copy_clone_marker]
    pub enum FontSize {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::fmt::Debug for FontSize {
        fn fmt(&self, f: &mut $crate::fmt::Formatter) -> $crate::fmt::Result {
            unsafe { $crate::intrinsics::unreachable() }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::marker::Copy for FontSize {}
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl $crate::clone::Clone for FontSize {
        #[inline]
        fn clone(&self) -> FontSize {
            {
                *self
            }
        }
    }
    impl<'i> FromCssDecl<'i> for FontSize {
        type Error = ParseError<'i, ()>;
        fn from_css_decl(
            _name: CowRcStr<'i>,
            input: &mut Parser<'i, '_>,
        ) -> Result<Self, ParseError<'i, ()>> {
            Err(input.new_custom_error(()))
        }
    }
}
