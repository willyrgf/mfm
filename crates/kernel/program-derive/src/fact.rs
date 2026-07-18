use super::*;

pub(super) fn derive(input: TokenStream) -> TokenStream {
    match expand_mfm_fact_type_derive_result(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_mfm_fact_type_derive_result(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "MfmFactType derive does not support generic structs in v1",
        ));
    }

    let attrs = FactContainerAttrs::parse(&input.attrs)?;
    validate_fact_container_attrs(&attrs)?;
    let fields = named_struct_fields(&input.data)?;
    let subject_field = fact_wrapper_field(fields, FactWrapperFieldRole::Subject)?;
    let response_field = fact_wrapper_field(fields, FactWrapperFieldRole::Response)?;
    let subject_ident = subject_field
        .ident
        .as_ref()
        .expect("named_struct_fields returns named fields");
    let response_ident = response_field
        .ident
        .as_ref()
        .expect("named_struct_fields returns named fields");
    let subject_ty = &subject_field.ty;
    let response_ty = &response_field.ty;
    let ident = &input.ident;
    let fact_kind = attrs.kind;
    let field_tokens = attrs
        .fields
        .iter()
        .map(fact_field_descriptor_tokens)
        .collect::<syn::Result<Vec<_>>>()?;
    let ordering_tokens = attrs
        .orderings
        .iter()
        .map(fact_ordering_descriptor_tokens)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl ::mfm_program::MfmFactType for #ident {
            type Subject = #subject_ty;
            type Response = #response_ty;

            fn descriptor() -> ::mfm_program::facts::Result<::mfm_program::facts::FactDescriptor> {
                let subject_schema_id = <Self::Subject as ::mfm_values::MfmValue>::schema_id()
                    .map_err(|error| ::mfm_program::facts::FactError::descriptor(error.to_string()))?;
                let response_schema_id = <Self::Response as ::mfm_values::MfmValue>::schema_id()
                    .map_err(|error| ::mfm_program::facts::FactError::descriptor(error.to_string()))?;

                ::mfm_program::facts::FactDescriptor::new(
                    ::mfm_program::facts::FactKind::new(#fact_kind)?,
                    ::mfm_program::facts::fact_descriptor_schema_id()?,
                    subject_schema_id,
                    response_schema_id,
                    vec![#(#field_tokens),*],
                    vec![#(#ordering_tokens),*],
                )
            }

            fn subject(&self) -> &Self::Subject {
                &self.#subject_ident
            }

            fn response(&self) -> &Self::Response {
                &self.#response_ident
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactWrapperFieldRole {
    Subject,
    Response,
}

#[derive(Debug)]
struct FactContainerAttrs {
    kind: String,
    fields: Vec<FactFieldAttr>,
    orderings: Vec<FactOrderingAttr>,
}

impl FactContainerAttrs {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut kind = None;
        let mut fields = Vec::new();
        let mut orderings = Vec::new();

        for attr in attrs {
            if !attr.path().is_ident("mfm_fact") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("kind") {
                    kind = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("field") {
                    fields.push(FactFieldAttr::parse(meta)?);
                    Ok(())
                } else if meta.path.is_ident("ordering") {
                    orderings.push(FactOrderingAttr::parse(meta)?);
                    Ok(())
                } else {
                    Err(meta.error("unsupported #[mfm_fact(...)] container attribute"))
                }
            })?;
        }

        let kind = kind.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "MfmFactType derive requires #[mfm_fact(kind = \"...\")]",
            )
        })?;
        if fields.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "MfmFactType derive requires at least one #[mfm_fact(field(...))] descriptor field",
            ));
        }

        Ok(Self {
            kind,
            fields,
            orderings,
        })
    }
}

fn validate_fact_container_attrs(attrs: &FactContainerAttrs) -> syn::Result<()> {
    let mut seen_fields = Vec::new();
    for field in &attrs.fields {
        if seen_fields.contains(&field.id.as_str()) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType derive found duplicate fact field id {:?}",
                    field.id
                ),
            ));
        }
        seen_fields.push(field.id.as_str());

        validate_fact_field_attr(field)?;
    }

    if !attrs
        .fields
        .iter()
        .any(|field| field.source == FactFieldAttrSource::Subject)
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "MfmFactType derive requires at least one subject fact field",
        ));
    }
    if !attrs
        .fields
        .iter()
        .any(|field| field.source == FactFieldAttrSource::Subject && field.required)
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "MfmFactType derive requires at least one required subject field",
        ));
    }

    for ordering in &attrs.orderings {
        for term in &ordering.terms {
            let Some(field) = attrs.fields.iter().find(|field| field.id == term.field) else {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "MfmFactType ordering {:?} references unknown fact field {:?}",
                        ordering.name, term.field
                    ),
                ));
            };
            if !field.sortable {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "MfmFactType ordering {:?} references non-sortable fact field {:?}",
                        ordering.name, term.field
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn validate_fact_field_attr(field: &FactFieldAttr) -> syn::Result<()> {
    let mut seen_operators = Vec::new();
    for operator in &field.operators {
        if seen_operators.contains(&operator.as_str()) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType field {:?} repeats operator {:?}",
                    field.id, operator
                ),
            ));
        }
        seen_operators.push(operator.as_str());

        if !fact_value_type_supports_operator(&field.value_type, operator) {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    "MfmFactType field {:?} uses operator {:?} incompatible with value_type {:?}",
                    field.id, operator, field.value_type
                ),
            ));
        }
    }

    if field.sortable && !fact_value_type_is_sortable(&field.value_type) {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "MfmFactType field {:?} marks non-sortable value_type {:?} sortable",
                field.id, field.value_type
            ),
        ));
    }

    Ok(())
}

fn fact_value_type_supports_operator(value_type: &str, operator: &str) -> bool {
    if operator == "equal" {
        return true;
    }
    matches!(
        (value_type, operator),
        (
            "signed_integer" | "unsigned_integer" | "timestamp" | "decimal_string",
            "less_than" | "less_than_or_equal" | "greater_than" | "greater_than_or_equal"
        )
    )
}

fn fact_value_type_is_sortable(value_type: &str) -> bool {
    matches!(
        value_type,
        "signed_integer" | "unsigned_integer" | "timestamp" | "decimal_string"
    )
}

#[derive(Debug)]
struct FactFieldAttr {
    id: String,
    source: FactFieldAttrSource,
    path: Option<String>,
    metadata: Option<String>,
    value_type: String,
    operators: Vec<String>,
    exposure: String,
    unit: Option<String>,
    scale: Option<i16>,
    sortable: bool,
    required: bool,
}

impl FactFieldAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut id = None;
        let mut source = None;
        let mut path = None;
        let mut metadata = None;
        let mut value_type = None;
        let mut operators = Vec::new();
        let mut exposure = None;
        let mut unit = None;
        let mut scale = None;
        let mut sortable = false;
        let mut required = true;

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                id = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("source") {
                source = Some(FactFieldAttrSource::parse(
                    &meta.value()?.parse::<LitStr>()?.value(),
                    meta.path.span(),
                )?);
            } else if meta.path.is_ident("path") {
                path = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("metadata") {
                metadata = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("value_type") {
                value_type = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("operator") {
                operators.push(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("operators") {
                meta.parse_nested_meta(|operator| {
                    operators.push(operator.path.require_ident()?.to_string());
                    Ok(())
                })?;
            } else if meta.path.is_ident("exposure") {
                exposure = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("unit") {
                unit = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("scale") {
                scale = Some(
                    meta.value()?
                        .parse::<syn::LitInt>()?
                        .base10_parse::<i16>()?,
                );
            } else if meta.path.is_ident("sortable") {
                sortable = true;
            } else if meta.path.is_ident("optional") {
                required = false;
            } else if meta.path.is_ident("required") {
                required = true;
            } else {
                return Err(meta.error("unsupported mfm_fact field attribute"));
            }
            Ok(())
        })?;

        let id = id.ok_or_else(|| meta.error("mfm_fact field requires id = \"...\""))?;
        let source =
            source.ok_or_else(|| meta.error("mfm_fact field requires source = \"...\""))?;
        let value_type =
            value_type.ok_or_else(|| meta.error("mfm_fact field requires value_type = \"...\""))?;
        let exposure =
            exposure.ok_or_else(|| meta.error("mfm_fact field requires exposure = \"...\""))?;
        if operators.is_empty() {
            operators.push("equal".to_owned());
        }

        match source {
            FactFieldAttrSource::Subject | FactFieldAttrSource::Result => {
                if path.is_none() {
                    return Err(meta.error("subject/result fact fields require path = \"...\""));
                }
                if metadata.is_some() {
                    return Err(meta.error("subject/result fact fields cannot use metadata"));
                }
            }
            FactFieldAttrSource::Metadata => {
                if metadata.is_none() {
                    return Err(meta.error("metadata fact fields require metadata = \"...\""));
                }
                if path.is_some() {
                    return Err(meta.error("metadata fact fields cannot use path"));
                }
            }
        }

        Ok(Self {
            id,
            source,
            path,
            metadata,
            value_type,
            operators,
            exposure,
            unit,
            scale,
            sortable,
            required,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactFieldAttrSource {
    Subject,
    Result,
    Metadata,
}

impl FactFieldAttrSource {
    fn parse(value: &str, span: Span) -> syn::Result<Self> {
        match value {
            "subject" => Ok(Self::Subject),
            "result" => Ok(Self::Result),
            "metadata" => Ok(Self::Metadata),
            _ => Err(syn::Error::new(
                span,
                "fact field source must be subject, result, or metadata",
            )),
        }
    }
}

#[derive(Debug)]
struct FactOrderingAttr {
    name: String,
    terms: Vec<FactOrderingTermAttr>,
}

impl FactOrderingAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut name = None;
        let mut terms = Vec::new();

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                name = Some(meta.value()?.parse::<LitStr>()?.value());
                Ok(())
            } else if meta.path.is_ident("term") {
                terms.push(FactOrderingTermAttr::parse(meta)?);
                Ok(())
            } else {
                Err(meta.error("unsupported mfm_fact ordering attribute"))
            }
        })?;

        let name = name.ok_or_else(|| meta.error("mfm_fact ordering requires name = \"...\""))?;
        if terms.is_empty() {
            return Err(meta.error("mfm_fact ordering requires at least one term(...)"));
        }
        Ok(Self { name, terms })
    }
}

#[derive(Debug)]
struct FactOrderingTermAttr {
    field: String,
    direction: String,
    nulls: String,
    tie_breaker: bool,
}

impl FactOrderingTermAttr {
    fn parse(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<Self> {
        let mut field = None;
        let mut direction = None;
        let mut nulls = None;
        let mut tie_breaker = false;

        meta.parse_nested_meta(|meta| {
            if meta.path.is_ident("field") {
                field = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("direction") {
                direction = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("nulls") {
                nulls = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("tie_breaker") {
                tie_breaker = true;
            } else {
                return Err(meta.error("unsupported mfm_fact ordering term attribute"));
            }
            Ok(())
        })?;

        Ok(Self {
            field: field.ok_or_else(|| meta.error("ordering term requires field = \"...\""))?,
            direction: direction
                .ok_or_else(|| meta.error("ordering term requires direction = \"...\""))?,
            nulls: nulls.ok_or_else(|| meta.error("ordering term requires nulls = \"...\""))?,
            tie_breaker,
        })
    }
}

fn fact_wrapper_field(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::Token![,]>,
    role: FactWrapperFieldRole,
) -> syn::Result<&syn::Field> {
    let mut matches = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "MfmFactType requires named fields"))?;
        let explicit_role = fact_wrapper_field_role(&field.attrs)?;
        let default_role = match ident.to_string().as_str() {
            "subject" => Some(FactWrapperFieldRole::Subject),
            "response" => Some(FactWrapperFieldRole::Response),
            _ => None,
        };
        if explicit_role.or(default_role) == Some(role) {
            matches.push(field);
        }
    }

    match matches.as_slice() {
        [field] => Ok(*field),
        [] => Err(syn::Error::new(
            Span::call_site(),
            match role {
                FactWrapperFieldRole::Subject => {
                    "MfmFactType derive requires one subject field named subject or marked #[mfm_fact(subject)]"
                }
                FactWrapperFieldRole::Response => {
                    "MfmFactType derive requires one response field named response or marked #[mfm_fact(response)]"
                }
            },
        )),
        [first, ..] => Err(syn::Error::new(
            first.span(),
            match role {
                FactWrapperFieldRole::Subject => "MfmFactType derive found multiple subject fields",
                FactWrapperFieldRole::Response => {
                    "MfmFactType derive found multiple response fields"
                }
            },
        )),
    }
}

fn fact_wrapper_field_role(attrs: &[Attribute]) -> syn::Result<Option<FactWrapperFieldRole>> {
    let mut role = None;
    for attr in attrs {
        if !attr.path().is_ident("mfm_fact") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let next = if meta.path.is_ident("subject") {
                FactWrapperFieldRole::Subject
            } else if meta.path.is_ident("response") {
                FactWrapperFieldRole::Response
            } else {
                return Err(meta.error("fact wrapper fields support only #[mfm_fact(subject)] or #[mfm_fact(response)]"));
            };
            if role.replace(next).is_some() {
                return Err(meta.error("duplicate fact wrapper field role"));
            }
            Ok(())
        })?;
    }
    Ok(role)
}

fn fact_field_descriptor_tokens(field: &FactFieldAttr) -> syn::Result<proc_macro2::TokenStream> {
    let id = &field.id;
    let extraction = fact_field_extraction_tokens(field)?;
    let value_type = fact_value_type_tokens(&field.value_type, Span::call_site())?;
    let operators = field
        .operators
        .iter()
        .map(|operator| fact_operator_tokens(operator, Span::call_site()))
        .collect::<syn::Result<Vec<_>>>()?;
    let exposure = fact_exposure_tokens(&field.exposure, Span::call_site())?;
    let unit = field
        .unit
        .as_ref()
        .map(|unit| quote!(Some(::mfm_program::facts::FactUnit::new(#unit)?)))
        .unwrap_or_else(|| quote!(None));
    let scale = field
        .scale
        .map(|scale| quote!(Some(::mfm_program::facts::FactScale::new(#scale)?)))
        .unwrap_or_else(|| quote!(None));
    let sortable = field.sortable;
    let required = field.required;

    Ok(quote! {
        ::mfm_program::facts::FactFieldDescriptor::new(
            ::mfm_program::facts::FactFieldId::new(#id)?,
            #value_type,
            #extraction,
            ::mfm_program::facts::FactFieldPolicy::new(vec![#(#operators),*], #exposure)
                .with_optional_unit(#unit)
                .with_optional_scale(#scale)
                .with_sortable(#sortable)
                .with_required(#required),
        )?
    })
}

fn fact_field_extraction_tokens(field: &FactFieldAttr) -> syn::Result<proc_macro2::TokenStream> {
    match field.source {
        FactFieldAttrSource::Subject => {
            let path = field.path.as_ref().expect("validated subject path");
            Ok(quote!(::mfm_program::facts::FactFieldExtraction::Subject(
                ::mfm_program::facts::CanonicalValuePath::new(#path)?
            )))
        }
        FactFieldAttrSource::Result => {
            let path = field.path.as_ref().expect("validated result path");
            Ok(quote!(::mfm_program::facts::FactFieldExtraction::Response(
                ::mfm_program::facts::CanonicalValuePath::new(#path)?
            )))
        }
        FactFieldAttrSource::Metadata => {
            let metadata = field.metadata.as_ref().expect("validated metadata field");
            let metadata = fact_metadata_field_tokens(metadata, Span::call_site())?;
            Ok(quote!(::mfm_program::facts::FactFieldExtraction::Metadata(#metadata)))
        }
    }
}

fn fact_ordering_descriptor_tokens(
    ordering: &FactOrderingAttr,
) -> syn::Result<proc_macro2::TokenStream> {
    let name = &ordering.name;
    let terms = ordering
        .terms
        .iter()
        .map(fact_ordering_term_tokens)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote! {
        ::mfm_program::facts::FactOrderingPolicy::new(
            ::mfm_program::facts::FactOrderingName::new(#name)?,
            vec![#(#terms),*],
        )?
    })
}

fn fact_ordering_term_tokens(term: &FactOrderingTermAttr) -> syn::Result<proc_macro2::TokenStream> {
    let field = &term.field;
    let direction = fact_sort_direction_tokens(&term.direction, Span::call_site())?;
    let nulls = fact_null_ordering_tokens(&term.nulls, Span::call_site())?;
    let tie_breaker = term.tie_breaker;
    Ok(quote! {
        ::mfm_program::facts::FactOrderingTerm::new(
            ::mfm_program::facts::FactFieldId::new(#field)?,
            #direction,
            #nulls,
            #tie_breaker,
        )
    })
}

fn fact_value_type_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "string" => Ok(quote!(::mfm_program::facts::FactFieldValueType::String)),
        "boolean" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Boolean)),
        "signed_integer" => Ok(quote!(::mfm_program::facts::FactFieldValueType::SignedInteger)),
        "unsigned_integer" => Ok(quote!(::mfm_program::facts::FactFieldValueType::UnsignedInteger)),
        "timestamp" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Timestamp)),
        "decimal_string" => Ok(quote!(::mfm_program::facts::FactFieldValueType::DecimalString)),
        "digest" => Ok(quote!(::mfm_program::facts::FactFieldValueType::Digest)),
        _ => Err(syn::Error::new(
            span,
            "fact value_type must be string, boolean, signed_integer, unsigned_integer, timestamp, decimal_string, or digest",
        )),
    }
}

fn fact_operator_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::Equal)),
        "less_than" => Ok(quote!(::mfm_program::facts::FactQueryOperator::LessThan)),
        "less_than_or_equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::LessThanOrEqual)),
        "greater_than" => Ok(quote!(::mfm_program::facts::FactQueryOperator::GreaterThan)),
        "greater_than_or_equal" => Ok(quote!(::mfm_program::facts::FactQueryOperator::GreaterThanOrEqual)),
        _ => Err(syn::Error::new(
            span,
            "fact operator must be equal, less_than, less_than_or_equal, greater_than, or greater_than_or_equal",
        )),
    }
}

fn fact_exposure_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "returnable" => Ok(quote!(::mfm_program::facts::FactFieldExposure::Returnable)),
        "query_only" => Ok(quote!(::mfm_program::facts::FactFieldExposure::QueryOnly)),
        "hidden" => Ok(quote!(::mfm_program::facts::FactFieldExposure::Hidden)),
        _ => Err(syn::Error::new(
            span,
            "fact exposure must be returnable, query_only, or hidden",
        )),
    }
}

fn fact_metadata_field_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "recorded_at" => Ok(quote!(::mfm_program::facts::FactMetadataField::RecordedAt)),
        "observed_at" => Ok(quote!(::mfm_program::facts::FactMetadataField::ObservedAt)),
        "store_commit_order" => Ok(quote!(
            ::mfm_program::facts::FactMetadataField::StoreCommitOrder
        )),
        _ => Err(syn::Error::new(
            span,
            "fact metadata must be recorded_at, observed_at, or store_commit_order",
        )),
    }
}

fn fact_sort_direction_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "ascending" => Ok(quote!(::mfm_program::facts::SortDirection::Ascending)),
        "descending" => Ok(quote!(::mfm_program::facts::SortDirection::Descending)),
        _ => Err(syn::Error::new(
            span,
            "fact ordering direction must be ascending or descending",
        )),
    }
}

fn fact_null_ordering_tokens(value: &str, span: Span) -> syn::Result<proc_macro2::TokenStream> {
    match value {
        "first" => Ok(quote!(::mfm_program::facts::NullOrdering::First)),
        "last" => Ok(quote!(::mfm_program::facts::NullOrdering::Last)),
        _ => Err(syn::Error::new(
            span,
            "fact ordering nulls must be first or last",
        )),
    }
}
