#![recursion_limit="128"]

extern crate proc_macro;

use inflector::cases::kebabcase::to_kebab_case;
use inflector::cases::snakecase::to_snake_case;
use quote::quote;
use syn::{Data, DataEnum, Field, Fields, FieldsUnnamed, Variant};
use proc_macro2::{Span, Ident};

#[proc_macro_derive(FromCssDecl, attributes(property, computed))]
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
    let computed_values = ast.attrs.iter().rev().filter_map(|attr| {
        match attr.parse_meta().expect("Could not parse attribute") {
            syn::Meta::NameValue(syn::MetaNameValue { ident, lit: syn::Lit::Str(lit), .. }) => {
                if ident == "computed" {
                    let tp = lit.parse::<syn::Type>().expect("Could not parse type");
                    Some(quote! { #tp })
                } else {
                    None
                }
            }
            _ => None,
        }
    }).next().unwrap_or(quote!{()});

    let variants = match &ast.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => panic!("#[FromCssDecl] can only be applied to enums."),
    };

    // let mut computed_value_members = vec![];
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
            let ident_kebab = to_kebab_case(&ident.to_string());
            let ident_snake = Ident::new(&to_snake_case(&ident.to_string()), Span::call_site());

            // let computed_value = quote! {
            //     pub #ident_snake: <#parser_ty as PropertyValue<'static>>::Computed
            // };
            // computed_value_members.push(computed_value);
            let some_ident = Ident::new("val", Span::call_site());
            property_cases.push(quote! { 
                #name::#ident(#some_ident) => {
                    computed.#ident_snake = #some_ident.computed_value();
                } 
            });

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

    // let computed_values = Ident::new(&format!("_Computed{}", name), Span::call_site());
    let trait_impl = quote! {
        impl<'i> FromCssDecl<'i> for #name {
            type Error = cssparser::ParseError<'i, ()>;
            type Computed = #computed_values;

            fn from_css_decl(name: cssparser::CowRcStr<'i>, input: &mut cssparser::Parser<'i, '_>) -> Result<#name, Self::Error> {
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

    // let members = computed_value_members.iter();
    // let computed_values = quote! {
    //     pub struct #computed_values {
    //         #(#members),*
    //     }
    // };

    let combined = quote! {
        #trait_impl 
        // #computed_values 
    };

    combined.into()
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
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) =>  {
                assert_eq!(unnamed.len(), 1, "Variant must not contain multiple fields.");
                let Field { ty, .. } = unnamed.into_iter().next().unwrap();
                quote! {
                    <#ty as PropertyValue>::from_css_property_value(input).map(#name::#ident)
                }  
            },
            Fields::Unit => {
                quote! {
                    input.expect_ident_matching(#ident_kebab).map(|()| #name::#ident)
                }
            },
            _ => unimplemented!()
        }
        
    });

    let gen = quote! {
        impl<'i> PropertyValue<'i> for #name {
            type Error = cssparser::ParseError<'i, ()>;

            fn from_css_property_value(input: &mut cssparser::Parser<'i, '_>) -> Result<#name, Self::Error> {
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
