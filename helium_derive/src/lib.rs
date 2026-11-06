#![recursion_limit = "128"]

extern crate proc_macro;

use inflector::cases::kebabcase::to_kebab_case;
use inflector::cases::pascalcase::to_pascal_case;
use inflector::cases::snakecase::to_snake_case;
use proc_macro2::{Ident, Span};
use quote::{quote, quote_spanned, ToTokens};
use syn::spanned::Spanned;
use syn::{Data, DataEnum, Field, Fields, FieldsUnnamed, LitStr, Variant};

#[proc_macro_derive(ParseDeclaration, attributes(property, ua_property, computed, shorthand, spec))]
pub fn parse_declaration_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = proc_macro2::TokenStream::from(input);

    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse2(input).unwrap();

    // Build the trait implementation
    let result = impl_parse_declaration_macro(&ast);

    result.into()
}

fn impl_parse_declaration_macro(ast: &syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &ast.ident;
    let generics = &ast.generics;
    let mut generics_appended = ast.generics.clone();
    generics_appended
        .params
        .push(syn::parse2(quote! { 'i }).unwrap());
    let computed_values_type = ast
        .attrs
        .iter()
        .rev()
        .filter_map(
            |attr| match attr.parse_meta().expect("Could not parse attribute") {
                syn::Meta::NameValue(syn::MetaNameValue {
                    path,
                    lit: syn::Lit::Str(lit),
                    ..
                }) => match path.get_ident() {
                    Some(ident) if ident == "computed" => {
                        let tp = lit.parse::<syn::Type>().expect("Could not parse type");
                        Some(quote! { #tp })
                    }
                    _ => None,
                },
                _ => None,
            },
        )
        .next()
        .unwrap_or(quote! {()});

    let variants = match &ast.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => panic!("#[derive(ParseDeclaration)] can only be applied to enums."),
    };

    let mut property_cases = vec![];
    // Cases of `CssWideProperties::apply_css_wide`, which resets a property by name.
    let mut css_wide_cases = vec![];

    let if_clauses = variants.into_iter().map(|Variant { ident, fields, attrs, .. }| {
        let mut variant_attribute = None;
        for attr in attrs {
            parse_variant_attribute(&mut variant_attribute, attr);
        }

        let parser_ty = match fields {
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) =>  {
                assert_eq!(unnamed.len(), 1, "Variant must not contain multiple fields.");
                let Field { ty, .. } = unnamed.into_iter().next().unwrap();
                ty
            },
            _ => panic!("Variant must not contain named fields.")
        };
        if variant_attribute.is_some() {
            let shorthands = parse_shorthands(&attrs);
            let shorthands_names = shorthands.iter().map(|sh| sh.property_name.value()).collect::<Vec<_>>();

            let ident_kebab = to_kebab_case(&ident.to_string());
            let ident_snake = Ident::new(&to_snake_case(&ident.to_string()),ident.span());

            if shorthands.is_empty() {
                property_cases.push(quote! {
                    #name::#ident(val) => {
                        let computed_value = val.computed_value(&*computed);
                        std::convert::AsMut::<#computed_values_type>::as_mut(computed).#ident_snake = computed_value;
                    }
                });
            } else {
                let shorthands = shorthands.iter().map(|sh| sh.token_stream(name));
                property_cases.push(quote! {
                    #name::#ident(val) => {
                        #(#shorthands);*
                    }
                });
            }

            let property_name = if variant_attribute == Some(VariantAttribute::UAProperty) {
                format!("-helium-{}", ident_kebab)
            } else {
                ident_kebab
            };

            if shorthands.is_empty() {
                css_wide_cases.push(quote! {
                    if name.eq_ignore_ascii_case(#property_name) {
                        let from_parent = match keyword {
                            CssWide::Inherit => true,
                            CssWide::Initial => false,
                            CssWide::Unset => is_inherited_property(#property_name),
                        };
                        let source: &#computed_values_type = if from_parent {
                            std::convert::AsRef::<#computed_values_type>::as_ref(parent)
                        } else {
                            std::convert::AsRef::<#computed_values_type>::as_ref(initial)
                        };
                        let value = source.#ident_snake.clone();
                        std::convert::AsMut::<#computed_values_type>::as_mut(computed).#ident_snake = value;
                        return true;
                    }
                });
            } else {
                // a shorthand resets its longhands
                let longhands = shorthands_names.clone();
                css_wide_cases.push(quote! {
                    if name.eq_ignore_ascii_case(#property_name) {
                        #(<Self as CssWideProperties>::apply_css_wide(#longhands, keyword, computed, parent, initial);)*
                        return true;
                    }
                });
            }

            quote! {
                if name.eq_ignore_ascii_case(#property_name) {
                    return <#parser_ty as PropertyValue>::from_css_property_value(input).map(#name::#ident);
                }
            }
        } else {
            css_wide_cases.push(quote! {
                if <#parser_ty as CssWideProperties>::apply_css_wide(name, keyword, computed, parent, initial) {
                    return true;
                }
            });
            property_cases.push(quote! {
                #name::#ident(val) => { val.apply_to(computed) }
            });
            quote! {
                match input.try_parse(|input| <#parser_ty as ParseDeclaration>::parse_declaration(name.clone(), input)) {
                    Ok(success) => return Ok(#name::#ident(success)),
                    Err(cssparser::ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::UnknownProperty { .. }), .. }) => {},
                    Err(other) => return Err(other)
                }
            }
        }
    }).collect::<Vec<_>>();

    let trait_impl = quote! {
        impl #generics_appended ParseDeclaration<'i> for #name #generics {
            type Error = StyleParseError;

            fn parse_declaration(name: cssparser::CowRcStr<'i>, input: &mut cssparser::Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
                #(#if_clauses)*
                Err(cssparser::ParseError::custom(StyleParseError::UnknownProperty { name: name.to_string() }))
            }
        }

        impl #generics CssWideProperties for #name #generics {
            fn apply_css_wide(
                name: &str,
                keyword: CssWide,
                computed: &mut super::computed::ComputedValues,
                parent: &super::computed::ComputedValues,
                initial: &super::computed::ComputedValues,
            ) -> bool {
                #(#css_wide_cases)*
                false
            }
        }

        impl #generics ApplyToComputedValues for #name #generics {
            fn apply_to(&self, computed: &mut super::computed::ComputedValues) {
                match self {
                    #(#property_cases),*
                }
            }
        }
    };

    trait_impl
}

#[derive(Debug, PartialEq, Eq)]
enum VariantAttribute {
    Property,
    // user agent  property
    UAProperty,
}

fn parse_variant_attribute(val: &mut Option<VariantAttribute>, attr: &syn::Attribute) {
    let meta = attr.parse_meta().unwrap();
    let attr_ident = meta.path().get_ident().unwrap();
    if attr_ident == "property" {
        *val = Some(VariantAttribute::Property)
    } else if attr_ident == "ua_property" {
        *val = Some(VariantAttribute::UAProperty)
    }
}

struct Shorthand {
    span: Span,
    property_name: LitStr,
    method_name: proc_macro2::Ident,
}

impl Shorthand {
    fn token_stream(&self, struct_name: &Ident) -> proc_macro2::TokenStream {
        let ident = Ident::new(
            &to_pascal_case(&self.property_name.value()),
            self.property_name.span(),
        );
        let method = &self.method_name;
        quote_spanned! { self.span =>
            #struct_name::#ident(val.#method()).apply_to(computed);
        }
    }
}

fn parse_shorthands(attrs: &[syn::Attribute]) -> Vec<Shorthand> {
    attrs
        .iter()
        .filter_map(|attr| {
            let meta = attr.parse_meta().unwrap();
            if meta.path().get_ident().unwrap() == "shorthand" {
                Some(meta)
            } else {
                None
            }
        })
        .map(|meta| match &meta {
            syn::Meta::List(syn::MetaList { nested, .. }) => {
                let mut iter = nested.iter();
                let property_name =
                    if let Some(syn::NestedMeta::Lit(syn::Lit::Str(str_lit))) = iter.next() {
                        str_lit.clone()
                    } else {
                        panic!("invalid property name")
                    };

                let span;
                let method_name = match iter.next() {
                    Some(syn::NestedMeta::Meta(syn::Meta::NameValue(name_value))) => {
                        assert!(name_value.path.get_ident().unwrap() == "method");
                        span = name_value.span();
                        if let syn::Lit::Str(str_lit) = &name_value.lit {
                            str_lit.parse().expect("Could not parse method name")
                        } else {
                            panic!("invalid method name {:?}", name_value.lit.to_token_stream())
                        }
                    }
                    other => panic!("invalid method name {:?}", other.to_token_stream()),
                };

                Shorthand {
                    property_name,
                    method_name,
                    span,
                }
            }
            _ => panic!("expected list in shorthand attribute"),
        })
        .collect()
}

// maybe used later
#[allow(unused)]
fn parse_spec(attrs: &[syn::Attribute]) -> Option<String> {
    let spec_attr = attrs
        .iter()
        .filter_map(|attr| {
            let meta = attr.parse_meta().unwrap();
            if meta.path().get_ident().unwrap() == "spec" {
                Some(meta)
            } else {
                None
            }
        })
        .next();
    spec_attr.map(|meta| match meta {
        syn::Meta::List(syn::MetaList { nested, .. }) => {
            match nested
                .into_iter()
                .next()
                .expect("#[spec] expects a string literal")
            {
                syn::NestedMeta::Lit(syn::Lit::Str(str_lit)) => str_lit.value(),
                _ => panic!("#[spec] expects a string literal"),
            }
        }
        _ => panic!("#[spec] expects a string literal"),
    })
}

/// Derives `PropertyValue` for an enum.
///
/// * Unit variants match a keyword (the kebab-cased variant name).
/// * Single-field tuple variants match a bare value of the field's type.
/// * Tuple variants annotated with `#[function]` match CSS function notation,
///   `name(arg arg ...)`. The function name is the kebab-cased variant name
///   unless overridden. Options (`#[function(comma, name = "foo")]`):
///   - `comma`: arguments are comma separated instead of whitespace separated
///   - `name = "..."`: override the function name
///
///   Each argument is parsed using its own `PropertyValue` impl. Trailing
///   fields of type `Option<T>` are optional arguments. The whole argument
///   list must be consumed.
#[proc_macro_derive(PropertyValue, attributes(function))]
pub fn property_value_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = proc_macro2::TokenStream::from(input);

    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse2(input).unwrap();

    // Build the trait implementation
    let result = impl_property_value_macro(&ast);

    result.into()
}

fn impl_property_value_macro(ast: &syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &ast.ident;

    let variants = match &ast.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => panic!("#[PropertyValue] can only be applied to enums."),
    };

    let sub_parsers = variants.into_iter().map(|Variant { ident, fields, attrs, .. }| {
        let ident_kebab = to_kebab_case(&ident.to_string());

        match fields {
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) if parse_function_attr(attrs).is_some() => {
                let FunctionAttr { name: fn_name, comma } = parse_function_attr(attrs).unwrap();
                let fn_name = fn_name.unwrap_or(ident_kebab);
                let mut seen_optional = false;
                let mut stmts = vec![];
                let mut vars = vec![];
                for (i, Field { ty, .. }) in unnamed.iter().enumerate() {
                    let var = Ident::new(&format!("__arg{}", i), ident.span());
                    match option_inner(ty) {
                        Some(inner) => {
                            seen_optional = true;
                            if comma && i > 0 {
                                stmts.push(quote! {
                                    let #var = if input.try_parse(|i| i.expect_comma()).is_ok() {
                                        Some(<#inner as PropertyValue>::from_css_property_value(input)?)
                                    } else {
                                        None
                                    };
                                });
                            } else {
                                stmts.push(quote! {
                                    let #var = input.try_parse(<#inner as PropertyValue>::from_css_property_value).ok();
                                });
                            }
                        }
                        None => {
                            assert!(!seen_optional, "Required function arguments must precede optional ones.");
                            if comma && i > 0 {
                                stmts.push(quote! { input.expect_comma()?; });
                            }
                            stmts.push(quote! {
                                let #var = <#ty as PropertyValue>::from_css_property_value(input)?;
                            });
                        }
                    }
                    vars.push(var);
                }
                quote! {
                    (|| -> Result<#name, cssparser::ParseError<StyleParseError>> {
                        input.expect_function_matching(#fn_name)?;
                        input.parse_nested_block(|input| -> Result<#name, cssparser::ParseError<StyleParseError>> {
                            #(#stmts)*
                            Ok(#name::#ident(#(#vars),*))
                        })
                    })()
                }
            }
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                assert_eq!(
                    unnamed.len(),
                    1,
                    "Variant must not contain multiple fields."
                );
                let Field { ty, .. } = unnamed.into_iter().next().unwrap();
                quote! {
                    <#ty as PropertyValue>::from_css_property_value(input).map(#name::#ident)
                }
            }
            Fields::Unit => {
                quote! {
                    input.expect_ident_matching(#ident_kebab).map(|()| #name::#ident)
                }
            }
            _ => unimplemented!(),
        }
    });

    let gen = quote! {
        impl<'i> PropertyValue<'i> for #name {
            type Error = StyleParseError;

            fn from_css_property_value(input: &mut cssparser::Parser<'i>) -> Result<#name, cssparser::ParseError<Self::Error>> {
                #(if let Ok(success) = input.try_parse(|input| #sub_parsers) {
                    return Ok(success)
                })*

                Err(input.new_error_for_next_token())
            }
        }
    };

    gen
}

struct FunctionAttr {
    name: Option<String>,
    comma: bool,
}

fn parse_function_attr(attrs: &[syn::Attribute]) -> Option<FunctionAttr> {
    let attr = attrs.iter().find(|attr| attr.path.is_ident("function"))?;
    let mut result = FunctionAttr {
        name: None,
        comma: false,
    };
    match attr.parse_meta().expect("Could not parse #[function] attribute") {
        syn::Meta::Path(_) => {}
        syn::Meta::List(syn::MetaList { nested, .. }) => {
            for item in nested {
                match item {
                    syn::NestedMeta::Meta(syn::Meta::Path(path)) if path.is_ident("comma") => {
                        result.comma = true
                    }
                    syn::NestedMeta::Meta(syn::Meta::NameValue(nv)) if nv.path.is_ident("name") => {
                        match nv.lit {
                            syn::Lit::Str(lit) => result.name = Some(lit.value()),
                            _ => panic!("#[function(name = ...)] expects a string literal"),
                        }
                    }
                    _ => panic!("unknown option in #[function(...)]; expected `comma` or `name = \"...\"`"),
                }
            }
        }
        _ => panic!("invalid #[function] attribute"),
    }
    Some(result)
}

/// If `ty` is `Option<T>`, returns `T`.
fn option_inner(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(syn::TypePath { qself: None, path }) = ty {
        let seg = path.segments.last()?;
        if seg.ident == "Option" {
            if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                    return Some(inner);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
