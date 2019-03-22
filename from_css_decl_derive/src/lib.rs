extern crate proc_macro;

use quote::{quote};
use syn::{Data, DataEnum, Variant, Fields, FieldsUnnamed, Field};
use inflector::cases::kebabcase::to_kebab_case;

#[proc_macro_derive(FromCssDecl)]
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

    let variants = match &ast.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => panic!("#[FromCssDecl] can only be applied to enums."),
    };

    let if_clauses = variants.into_iter().map(|Variant { ident, fields, .. }| {
        let ident_kebab = to_kebab_case(&ident.to_string());
        let parser_ty = match fields {
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) =>  {
                assert_eq!(unnamed.len(), 1, "Variant must not contain multiple fields.");
                let Field { ty, .. } = unnamed.into_iter().next().unwrap();
                ty
            },
            _ => panic!("Variant must not contain named fields.")
        };
        quote! {
            if name.eq_ignore_ascii_case(#ident_kebab) {
                return <#parser_ty as FromCssDecl>::from_css_decl(name, input).map(#name::#ident);
            }
        }
    });

    let gen = quote! {
        impl<'i> FromCssDecl<'i> for #name {
            type Error = cssparser::ParseError<'i, ()>;

            fn from_css_decl(name: cssparser::CowRcStr<'i>, input: &mut cssparser::Parser<'i, '_>) -> Result<#name, Self::Error> {
                #(#if_clauses)*
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
