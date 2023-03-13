use proc_macro2::TokenStream;
use syn::{DeriveInput, Expr, Ident, LitStr, Type};

use attrs::AttributeSpanWrapper;
use field::{Field, FieldName};
use model::Model;
use util::wrap_in_dummy_mod;

pub fn derive(item: DeriveInput) -> TokenStream {
    let model = Model::from_item(&item, false, false);

    let struct_name = &item.ident;
    let fields = &model.fields().iter().map(get_ident).collect::<Vec<_>>();
    let field_names = model.fields().iter().map(|f| &f.name);

    let initial_field_expr = model.fields().iter().map(|f| {
        let field_ty = &f.ty;

        if f.embed() {
            quote!(<#field_ty as QueryableByName<__DB>>::build(row)?)
        } else {
            let deserialize_ty = f.ty_for_deserialize();
            let name = f.column_name();
            let name = LitStr::new(&name.to_string(), name.span());
            quote!(
               {
                   let field = NamedRow::get(row, #name)?;
                   <#deserialize_ty as Into<#field_ty>>::into(field)
               }
            )
        }
    });


    let (_, ty_generics, ..) = item.generics.split_for_impl();
    let mut generics = item.generics.clone();
    generics
        .params
        .push(parse_quote!(__DB: backend::Backend));

    let mut include_table_def = false;
    for field in model.fields() {
        let where_clause = generics.where_clause.get_or_insert(parse_quote!(where));
        let field_ty = field.ty_for_deserialize();
        if field.embed() {
            where_clause
                .predicates
                .push(parse_quote!(#field_ty: QueryableByName<__DB>));
        } else {
            let st = sql_type(field, &model, &mut include_table_def);
            where_clause
                .predicates
                .push(parse_quote!(#field_ty: deserialize::FromSql<#st, __DB>));
        }
    }

    let (impl_generics, _, where_clause) = generics.split_for_impl();

    // include_table_def is to have a bit better errors in compile time
    // in case of a typo in the table_name or missing of table_name attribute
    let table_name = &model.table_names()[0];
    let include_table_def: Option<Expr> = if include_table_def {
        Some(parse_quote!({use self::#table_name::table;}))
    } else {
        None
    };

    wrap_in_dummy_mod(quote! {
        use diesel::deserialize::{self, QueryableByName};
        use diesel::row::{NamedRow};
        use diesel::sql_types::Untyped;
        #include_table_def

        impl #impl_generics QueryableByName<__DB>
            for #struct_name #ty_generics
        #where_clause
        {
            fn build<'__a>(row: &impl NamedRow<'__a, __DB>) -> deserialize::Result<Self>
            {
                #(
                    let mut #fields = #initial_field_expr;
                )*
                Ok(Self {
                    #(
                        #field_names: #fields,
                    )*
                })
            }
        }
    })
}

fn get_ident(field: &Field) -> Ident {
    match &field.name {
        FieldName::Named(n) => n.clone(),
        FieldName::Unnamed(i) => Ident::new(&format!("field_{}", i.index), i.span),
    }
}

fn sql_type(field: &Field, model: &Model, include_table_def: &mut bool) -> Type {
    let table_name = &model.table_names()[0];

    match field.sql_type {
        Some(AttributeSpanWrapper { item: ref st, .. }) => st.clone(),
        None => {
            let column_name = field.column_name();
            *include_table_def = true;
            parse_quote!(dsl::SqlTypeOf<#table_name::#column_name>)
        }
    }
}
