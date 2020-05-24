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

            quote! {
                if name.eq_ignore_ascii_case(#property_name) {
                    return <#parser_ty as PropertyValue>::from_css_property_value(input).map(#name::#ident);
                }
            }
        } else {
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
            type Error = StyleParseError<'i>;

            fn parse_declaration(name: cssparser::CowRcStr<'i>, input: &mut cssparser::Parser<'i, '_>) -> Result<Self, ParseError<'i, Self::Error>> {
                #(#if_clauses)*
                Err(input.new_custom_error(StyleParseError::UnknownProperty { name }))
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
            type Error = StyleParseError<'i>;

            fn from_css_property_value(input: &mut cssparser::Parser<'i, '_>) -> Result<#name, cssparser::ParseError<'i, Self::Error>> {
                #(if let Ok(success) = input.r#try(|input| #sub_parsers) {
                    return Ok(success)
                })*

                Err(input.new_error_for_next_token())
            }
        }
    };

    gen
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
