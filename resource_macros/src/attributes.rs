pub mod derive_attr {
    use bae::FromAttributes;

    #[derive(Debug, FromAttributes)]
    pub struct Resource {
        pub schema_name: Option<syn::Lit>,
        pub pg_table_name: syn::Lit,
        pub sqlite_table_name: syn::Lit,
        pub constraint: syn::Lit,
        pub primary_key: syn::Lit,
        pub table_iden: Option<()>,
    }
}

pub mod field_attr {
    use bae::FromAttributes;

    #[derive(Debug, Default, FromAttributes)]
    pub struct Resource {
        pub name: Option<syn::Lit>,
        pub typ: Option<syn::Lit>,
        pub fields: Option<syn::Lit>,
    }
}
