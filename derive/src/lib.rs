use proc_macro2::{Span, TokenStream};
use quote::quote;
use serde_derive_internals::{
    attr::{Container, Default as SerdeDefault, Field},
    Ctxt,
};
use syn::{parse_macro_input, Data, DataStruct, DeriveInput, Error, Fields, Lifetime, Result};

#[cfg(test)]
mod tests;

fn column_names(data: &DataStruct, cx: &Ctxt, container: &Container) -> Result<TokenStream> {
    Ok(match &data.fields {
        Fields::Named(fields) => {
            let rename_rule = container.rename_all_rules().deserialize;
            let column_names_iter = fields
                .named
                .iter()
                .enumerate()
                .map(|(index, field)| Field::from_ast(cx, index, field, None, &SerdeDefault::None))
                .filter(|field| !field.skip_serializing() && !field.skip_deserializing())
                .map(|field| {
                    rename_rule
                        .apply_to_field(field.name().serialize_name())
                        .to_string()
                });

            quote! {
                &[#( #column_names_iter,)*]
            }
        }
        Fields::Unnamed(_) => {
            quote! { &[] }
        }
        Fields::Unit => unreachable!("checked by the caller"),
    })
}

// TODO: support wrappers `Wrapper(Inner)` and `Wrapper<T>(T)`.
// TODO: support the `nested` attribute.
// TODO: support the `crate` attribute.
#[proc_macro_derive(Row)]
pub fn row(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    row_impl(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn row_impl(input: DeriveInput) -> Result<TokenStream> {
    let cx = Ctxt::new();
    let container = Container::from_ast(&cx, &input);
    let name = input.ident;

    let result = match &input.data {
        Data::Struct(data) if data.fields.is_empty() => {
            let reason = "`Row` cannot be derived for unit or empty structs";
            Err(Error::new(name.span(), reason))
        }
        Data::Struct(data) => column_names(data, &cx, &container),
        Data::Enum(_) | Data::Union(_) => {
            let reason = "`Row` can only be derived for structs";
            Err(Error::new(name.span(), reason))
        }
    };

    cx.check()?;
    let column_names = result?;

    let value = match input.generics.lifetimes().count() {
        // An owned row: `struct Row { .. }`
        0 => quote! { Self },
        // A borrowed row: `struct Row<'a> { .. }`
        1 => {
            // Replace the lifetime with `__v` to set `Value<'__v> = ..`.
            let mut cloned = input.generics.clone();
            let param = cloned.lifetimes_mut().next().unwrap();
            param.lifetime = Lifetime::new("'__v", Span::call_site());
            let ty_generics = cloned.split_for_impl().1;
            quote! { #name #ty_generics }
        }
        // A borrowed row with multiple lifetimes: `struct Row<'a, 'b> { .. }`
        _ => {
            let lt = input.generics.lifetimes().nth(1).unwrap();
            let reason = "`Row` cannot be derived for structs with multiple lifetimes";
            return Err(Error::new(lt.lifetime.span(), reason));
        }
    };

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // TODO: replace `clickhouse` with `::clickhouse` here.
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics clickhouse::Row for #name #ty_generics #where_clause {
            const NAME: &'static str = stringify!(#name);
            const COLUMN_NAMES: &'static [&'static str] = #column_names;
            const COLUMN_COUNT: usize = <Self as clickhouse::Row>::COLUMN_NAMES.len();
            const KIND: clickhouse::_priv::RowKind = clickhouse::_priv::RowKind::Struct;

            type Value<'__v> = #value;
        }
    })
}
