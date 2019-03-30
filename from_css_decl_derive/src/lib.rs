#![recursion_limit = "128"]

extern crate proc_macro;

use inflector::cases::kebabcase::to_kebab_case;
use inflector::cases::snakecase::to_snake_case;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::{Data, DataEnum, Field, Fields, FieldsUnnamed, Variant};

#[proc_macro_derive(FromCssDecl, attributes(property, computed, shorthand, spec))]
pub fn from_css_decl_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = proc_macro2::TokenStream::from(input);

    // Construct a representation of Rust code as a syntax tree
    // that we can manipulate
    let ast = syn::parse2(input).unwrap();

    // Build the trait implementation
    let result = impl_from_css_decl_macro(&ast);

    result.into()
}

fn impl_from_css_decl_macro(ast: &syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &ast.ident;
    let computed_values = ast
        .attrs
        .iter()
        .rev()
        .filter_map(
            |attr| match attr.parse_meta().expect("Could not parse attribute") {
                syn::Meta::NameValue(syn::MetaNameValue {
                    ident,
                    lit: syn::Lit::Str(lit),
                    ..
                }) => {
                    if ident == "computed" {
                        let tp = lit.parse::<syn::Type>().expect("Could not parse type");
                        Some(quote! { #tp })
                    } else {
                        None
                    }
                }
                _ => None,
            },
        )
        .next()
        .unwrap_or(quote! {()});

    let variants = match &ast.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => panic!("#[FromCssDecl] can only be applied to enums."),
    };

    let mut property_cases = vec![];

    let if_clauses = variants.into_iter().map(|Variant { ident, fields, attrs, .. }| {
        let is_property = attrs.iter().any(|attr| attr.parse_meta().unwrap().name() == "property");

        let parser_ty = match fields {
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) =>  {
                assert_eq!(unnamed.len(), 1, "Variant must not contain multiple fields.");
                let Field { ty, .. } = unnamed.into_iter().next().unwrap();
                ty
            },
            _ => panic!("Variant must not contain named fields.")
        };
        if is_property {
            let shorthands = parse_shorthands(&attrs);
            let spec = parse_spec(&attrs);

            let ident_kebab = to_kebab_case(&ident.to_string());
            let ident_snake = Ident::new(&to_snake_case(&ident.to_string()), Span::call_site());

            if shorthands.is_empty() {
                property_cases.push(quote! {
                    #name::#ident(val) => {
                        computed.#ident_snake = val.computed_value();
                    }
                });
            } else {
                let shorthands = shorthands.iter().map(|sh| sh.token_stream());
                property_cases.push(quote! {
                    #name::#ident(val) => {
                        #(#shorthands);*
                    }
                });
            }

            quote! {
                if name.eq_ignore_ascii_case(#ident_kebab) {
                    return <#parser_ty as PropertyValue>::from_css_property_value(input).map(#name::#ident);
                }
            }
        } else {
            quote! {
                match <#parser_ty as FromCssDecl>::from_css_decl(name.clone(), input) {
                    Ok(success) => return Ok(#name::#ident(success)),
                    Err(err) => {},
                }
            }
        }
    }).collect::<Vec<_>>();

    let trait_impl = quote! {
        impl<'i> FromCssDecl<'i> for #name {
            type Error = ();
            type Computed = #computed_values;

            fn from_css_decl(name: cssparser::CowRcStr<'i>, input: &mut cssparser::Parser<'i, '_>) -> Result<#name, ParseError<'i, Self::Error>> {
                #(#if_clauses)*
                Err(input.new_custom_error(()))
            }

            fn apply_to(&self, computed: &mut #computed_values) {
                match self {
                    #(#property_cases),*
                    _ => {}
                }
            }
        }
    };

    trait_impl.into()
}

struct Shorthand {
    property_name: String,
    method_name: proc_macro2::Ident,
}

impl Shorthand {
    fn token_stream(&self) -> proc_macro2::TokenStream {
        let ident = Ident::new(&to_snake_case(&self.property_name), Span::call_site());
        let method = &self.method_name;
        quote! {
            computed.#ident = val.#method().computed_value()
        }
    }
}

fn parse_shorthands(attrs: &[syn::Attribute]) -> Vec<Shorthand> {
    attrs
        .iter()
        .filter_map(|attr| {
            let meta = attr.parse_meta().unwrap();
            if meta.name() == "shorthand" {
                Some(meta)
            } else {
                None
            }
        })
        .map(|meta| match meta {
            syn::Meta::List(syn::MetaList { nested, .. }) => {
                let mut iter = nested.into_iter();
                let property_name =
                    if let Some(syn::NestedMeta::Literal(syn::Lit::Str(str_lit))) = iter.next() {
                        str_lit.value()
                    } else {
                        panic!("invalid property name")
                    };

                let method_name = match iter.next() {
                    Some(syn::NestedMeta::Meta(syn::Meta::NameValue(name_value))) => {
                        assert!(name_value.ident == "method");
                        if let syn::Lit::Str(str_lit) = name_value.lit {
                            str_lit.parse().expect("Could not parse method name")
                        } else {
                            panic!("invalid method name {:?}", name_value.lit)
                        }
                    }
                    other => panic!("invalid method name {:?}", other),
                };

                Shorthand {
                    property_name,
                    method_name,
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
            if meta.name() == "spec" {
                Some(meta)
            } else {
                None
            }
        })
        .next();
    spec_attr.map(|meta| {
        match meta {
            syn::Meta::List(syn::MetaList { nested, .. }) => {
                match nested.into_iter().next().expect("#[spec] expects a string literal") {
                    syn::NestedMeta::Literal(syn::Lit::Str(str_lit)) => {
                        str_lit.value()
                    }
                    _ => panic!("#[spec] expects a string literal")
                }
            }
            _ => panic!("#[spec] expects a string literal"),
        }
    })
}

#[proc_macro_derive(PropertyValue)]
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

    let sub_parsers = variants.into_iter().map(|Variant { ident, fields, .. }| {
        let ident_kebab = to_kebab_case(&ident.to_string());

        match fields {
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
            type Error = ();

            fn from_css_property_value(input: &mut cssparser::Parser<'i, '_>) -> Result<#name, cssparser::ParseError<'i, Self::Error>> {
                #(if let Ok(success) = input.r#try(|input| #sub_parsers) {
                    return Ok(success)
                })*

                Err(input.new_custom_error(()))
            }
        }
    };

    gen.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
