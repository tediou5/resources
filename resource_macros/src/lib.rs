#![feature(let_chains, iter_intersperse)]
extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput, Error};

mod attributes;

#[proc_macro_derive(Resource, attributes(resource))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_derive_entity(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand_derive_entity(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    Ok(DeriveResource::new(input)?.expand())
}

#[derive(Debug, Clone)]
struct Field {
    ident: syn::Ident,
    name: String,
    typ: Option<String>,
}

#[derive(Debug)]
struct DeriveResource {
    struct_ident: syn::Ident,
    struct_generics: syn::Generics,
    schema_name: Option<syn::Ident>,
    pg_table_name: String,
    sqlite_table_name: String,
    primary_keys: Vec<(syn::Ident, syn::Ident)>,
    constraint: String,
    fields: Vec<Field>,
    error: syn::Ident,
}

impl DeriveResource {
    fn new(input: syn::DeriveInput) -> Result<Self, syn::Error> {
        let attributes::derive_attr::Resource {
            schema_name,
            pg_table_name,
            sqlite_table_name,
            constraint,
            primary_key,
            table_iden: _,
            error,
        } = attributes::derive_attr::Resource::from_attributes(&input.attrs)?;
        let struct_ident = input.ident;
        let struct_generics = input.generics;

        let schema_name = schema_name
            .and_then(|s| parse_lit_string(&s).ok())
            .map(|s| format_ident!("{s}"));
        let error = error
            .and_then(|s| parse_lit_string(&s).ok())
            .map(|s| format_ident!("{s}"))
            .unwrap_or(format_ident!("resource"));
        let pg_table_name = parse_lit_string(&pg_table_name)?.to_string();
        let sqlite_table_name = parse_lit_string(&sqlite_table_name)?.to_string();
        let constraint = parse_lit_string(&(constraint))?.to_string();
        let pkey = parse_lit_string(&(primary_key))?;
        let mut pkey = pkey.to_string();
        pkey.retain(|c| c != ' ');
        let primary_keys: Vec<(syn::Ident, syn::Ident)> = pkey
            .as_str()
            .split(',')
            .map(|s| s.split_once(':').unwrap())
            .map(|(n, ty)| (format_ident!("{n}"), format_ident!("{ty}")))
            .collect();

        let fields: Vec<Field> = if let syn::Data::Struct(item_struct) = input.data
            && let syn::Fields::Named(fields) = item_struct.fields
        {
            fields
                .named
                .iter()
                .filter_map(|field| {
                    let ident = field.ident.as_ref()?;
                    let field_attr =
                        attributes::field_attr::Resource::try_from_attributes(&field.attrs).ok()?;
                    let (name, typ) = if let Some(attr) = field_attr
                        && let Some(name) = attr.name
                    {
                        let name = parse_lit_string(&name).unwrap().to_string();
                        let typ = attr
                            .typ
                            .and_then(|t| parse_lit_string(&t).ok())
                            .map(|t| t.to_string());
                        (name, typ)
                    } else {
                        let original_field_name = trim_starting_raw_identifier(ident);
                        use heck::ToSnakeCase as _;
                        let name = original_field_name.as_str().to_snake_case();
                        (name, None)
                    };
                    Some(Field {
                        name,
                        typ,
                        ident: ident.clone(),
                    })
                })
                .collect()
        } else {
            vec![]
        };

        Ok(DeriveResource {
            struct_ident,
            struct_generics,
            schema_name,
            pg_table_name,
            sqlite_table_name,
            primary_keys,
            constraint,
            fields,
            error,
        })
    }

    fn gen_upsert(&self) -> (String, String, String, String, String, String) {
        let Self {
            struct_ident: _,
            struct_generics: _,
            schema_name,
            pg_table_name,
            sqlite_table_name,
            primary_keys,
            constraint,
            fields,
            error: _,
        } = self;

        let mut pk_fields: Vec<Field> = primary_keys
            .clone()
            .into_iter()
            .map(|(ident, _)| Field {
                name: ident.to_string(),
                ident,
                typ: None,
            })
            .collect();

        let upsert_set: String = fields
            .iter()
            .map(|f| format!("{} = EXCLUDED.{}", f.name, f.name))
            .intersperse(", ".to_string())
            .collect();

        let mut fields = fields.clone();

        pk_fields.append(&mut fields);
        let fields = pk_fields;

        let pg_table_name = if let Some(schema) = schema_name {
            format!("{schema}.{pg_table_name}")
        } else {
            pg_table_name.clone()
        };

        let select: String = fields
            .iter()
            .map(|f| f.name.to_string())
            .intersperse(", ".to_string())
            .collect();

        let pg_values: String = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut v = format!("${}", i + 1);
                if let Some(typ) = &f.typ {
                    let typ = format!("::{typ}");
                    let typ = typ.as_str();
                    v.push_str(typ);
                };
                v
            })
            .intersperse(", ".to_string())
            .collect();
        let sqlite_values: String = fields
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .intersperse(", ".to_string())
            .collect();

        let del: String = primary_keys
            .iter()
            .enumerate()
            .map(|(i, (f, _ty))| format!("{f} =${}", i + 1))
            .intersperse(" AND ".to_string())
            .collect();

        let pkey_constraint: String = primary_keys
            .iter()
            .map(|(f, _ty)| format!("{f}"))
            .intersperse(", ".to_string())
            .collect();

        let pg_insert = format!("INSERT INTO {pg_table_name} ( {select} ) VALUES ( {pg_values} )");
        let sqlite_insert =
            format!("INSERT INTO {sqlite_table_name} ( {select} ) VALUES ( {sqlite_values} )");
        let pg_upsert = format!(
            "{pg_insert} ON CONFLICT ON CONSTRAINT {constraint} DO UPDATE SET {upsert_set}"
        );
        let sqlite_upsert =
            format!("{sqlite_insert} ON CONFLICT ({pkey_constraint}) DO UPDATE SET {upsert_set}");
        let pg_delete = format!("DELETE FROM {pg_table_name} WHERE {del}");
        let sqlite_delete = format!("DELETE FROM {sqlite_table_name} WHERE {del}");
        (
            pg_insert,
            sqlite_insert,
            pg_upsert,
            sqlite_upsert,
            pg_delete,
            sqlite_delete,
        )
    }

    fn expand(&self) -> proc_macro2::TokenStream {
        let Self {
            struct_ident,
            struct_generics,
            schema_name: _,
            pg_table_name: _,
            sqlite_table_name: _,
            primary_keys,
            constraint: _,
            fields,
            error,
        } = self;

        let (_, ty_generics, where_clause) = struct_generics.split_for_impl();

        let (_pg_insert, sqlite_insert, _pg_upsert, sqlite_upsert, _pg_delete, sqlite_delete) =
            self.gen_upsert();

        let mut primary_keys_c = primary_keys.clone();
        // println!("pkeys: {primary_keys:#?}");
        let (ids, ids_typ) = match primary_keys.len() {
            0 => (quote! { () }, quote! { () }),
            1 => {
                let (id, typ) = primary_keys_c.remove(0);
                (quote! { #id }, quote! { #typ })
            }
            _ => {
                let (id1, typ1) = primary_keys_c.remove(0);
                let ids: Vec<&syn::Ident> = primary_keys_c.iter().map(|(id, _typ)| id).collect();
                let typs: Vec<&syn::Ident> = primary_keys_c.iter().map(|(_id, typ)| typ).collect();
                (
                    quote! { (#id1 #(, #ids)* ) },
                    quote! { (#typ1 #(, #typs)* ) },
                )
            }
        };

        let bind_pks = match primary_keys.len() {
            0 => quote!(),
            _ => {
                let pks: Vec<&syn::Ident> = primary_keys.iter().map(|(id, _typ)| id).collect();
                quote! { #(.bind(&#pks))* }
            }
        };

        let bind_fields = match fields.len() {
            0 => quote!(),
            _ => {
                let fs: Vec<&syn::Ident> = fields.iter().map(|f| &f.ident).collect();
                quote! { #(.bind(&self.#fs))* }
            }
        };

        {
            // let impl_pg_res = quote! {
            //         #[automatically_derived]
            //         impl #ty_generics Resource<Postgres> for #struct_ident #ty_generics #where_clause {
            //             type ResourceID = #ids_typ;
            //             async fn insert<'c, E>(
            //                 &self,
            //                 id: &Option<Self::ResourceID>,
            //                 exector: E,
            //             ) -> Result<(), #error::Error>
            //             where
            //                 E: sqlx::Executor<'c, Database = Any>,
            //             {
            //                 let #ids = if let Some(#ids) = id.clone() {
            //                     #ids
            //                 } else {
            //                     <Self as GenResourceID>::gen_id().await?
            //                 };

            //                 sqlx::query(#pg_insert)
            //                 #bind_pks
            //                 #bind_fields
            //                 .execute(exector)
            //                 .await?;
            //             Ok(())
            //             }

            //             async fn upsert<'c, E>(&self, id: &Option<Self::ResourceID>, exector: E) -> Result<(), #error::Error>
            //             where
            //                 E: sqlx::Executor<'c, Database = Any>,
            //             {
            //                 let #ids = if let Some(#ids) = id.clone() {
            //                     #ids
            //                 } else {
            //                     <Self as GenResourceID>::gen_id().await?
            //                 };

            //                 sqlx::query(#pg_upsert)
            //                 #bind_pks
            //                 #bind_fields
            //                 .execute(exector)
            //                 .await?;
            //             Ok(())
            //             }

            //             async fn update<'c, E>(&self, id: &Self::ResourceID, exector: E) -> Result<(), #error::Error>
            //             where
            //             E: sqlx::Executor<'c, Database = Any>,
            //             {
            //                 let #ids = id.clone();

            //                 sqlx::query(#pg_upsert)
            //                 #bind_pks
            //                 #bind_fields
            //                 .execute(exector)
            //                 .await?;
            //             Ok(())
            //             }

            //             async fn drop<'c, E>(id: &Self::ResourceID, exector: E) -> Result<(), #error::Error>
            //             where
            //             E: sqlx::Executor<'c, Database = Any>,
            //             {
            //                 let #ids = id.clone();

            //                 sqlx::query(#pg_delete)
            //                 #bind_pks
            //                 .execute(exector)
            //                 .await?;
            //             Ok(())
            //             }
            //         }
            //     };
        }
        let impl_sqlite_res = quote! {
            #[automatically_derived]
            impl #ty_generics Resource<Sqlite> for #struct_ident #ty_generics #where_clause {
                type ResourceID = #ids_typ;
                async fn insert<'c, E>(&self, id: &Option<Self::ResourceID>, exector: E) -> Result<(), #error::Error>
                where
                E: sqlx::Executor<'c, Database = Sqlite>,
                {
                    let #ids = if let Some(#ids) = id.clone() {
                        #ids
                    } else {
                        <Self as GenResourceID>::gen_id().await?
                    };

                    sqlx::query(#sqlite_insert)
                    #bind_pks
                    #bind_fields
                    .execute(exector)
                    .await?;
                Ok(())
                }

                async fn upsert<'c, E>(&self, id: &Option<Self::ResourceID>, exector: E) -> Result<(), #error::Error>
                where
                E: sqlx::Executor<'c, Database = Sqlite>,
                {
                    let #ids = if let Some(#ids) = id.clone() {
                        #ids
                    } else {
                        <Self as GenResourceID>::gen_id().await?
                    };

                    sqlx::query(#sqlite_upsert)
                    #bind_pks
                    #bind_fields
                    .execute(exector)
                    .await?;
                Ok(())
                }

                async fn update<'c, E>(&self, id: &Self::ResourceID, exector: E) -> Result<(), #error::Error>
                where
                E: sqlx::Executor<'c, Database = Sqlite>,
                {
                    let #ids = id.clone();

                    sqlx::query(#sqlite_upsert)
                    #bind_pks
                    #bind_fields
                    .execute(exector)
                    .await?;
                Ok(())
                }

                async fn drop<'c, E>(id: &Self::ResourceID, exector: E) -> Result<(), #error::Error>
                where
                E: sqlx::Executor<'c, Database = Sqlite>,
                {
                    let #ids = id.clone();

                    sqlx::query(#sqlite_delete)
                    #bind_pks
                    .execute(exector)
                    .await?;
                Ok(())
                }
            }
        };

        proc_macro2::TokenStream::from_iter([impl_sqlite_res])
        // proc_macro2::TokenStream::from_iter([impl_pg_res, impl_sqlite_res])
        // impl_pg_res
    }
}

fn parse_lit_string(lit: &syn::Lit) -> syn::Result<TokenStream> {
    match lit {
        syn::Lit::Str(lit_str) => lit_str
            .value()
            .parse()
            .map_err(|_| syn::Error::new_spanned(lit, "attribute not valid")),
        _ => Err(syn::Error::new_spanned(lit, "attribute must be a string")),
    }
}

pub(crate) const RAW_IDENTIFIER: &str = "r#";

pub(crate) fn trim_starting_raw_identifier<T>(string: T) -> String
where
    T: ToString,
{
    string
        .to_string()
        .trim_start_matches(RAW_IDENTIFIER)
        .to_string()
}
