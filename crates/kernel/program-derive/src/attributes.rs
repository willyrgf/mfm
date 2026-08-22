use super::*;

#[derive(Debug)]
pub(super) struct ContainerAttrs {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) version: String,
    pub(super) schema_name: String,
    pub(super) rename_all: Option<String>,
    pub(super) enum_tag: Option<String>,
    pub(super) enum_content: Option<String>,
    pub(super) transparent_string: bool,
    pub(super) transparent_bytes: bool,
    pub(super) transparent_map: bool,
    pub(super) serde_transparent: bool,
    pub(super) unsigned_minimum: Option<u64>,
    pub(super) unsigned_maximum: Option<u64>,
}

impl ContainerAttrs {
    pub(super) fn parse(attrs: &[Attribute], ident: &Ident) -> syn::Result<Self> {
        let type_name = snake_case(&ident.to_string());
        let mut output = Self {
            namespace: "mfm.derived".to_owned(),
            name: type_name.clone(),
            version: "1".to_owned(),
            schema_name: format!("mfm.derived.{type_name}"),
            rename_all: None,
            enum_tag: None,
            enum_content: None,
            transparent_string: false,
            transparent_bytes: false,
            transparent_map: false,
            serde_transparent: false,
            unsigned_minimum: None,
            unsigned_maximum: None,
        };

        for attr in attrs {
            if attr.path().is_ident("mfm") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("namespace") {
                        output.namespace = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("name") {
                        output.name = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("version") {
                        output.version = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("schema") {
                        output.schema_name = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.path.is_ident("transparent_string") {
                        output.transparent_string = true;
                    } else if meta.path.is_ident("transparent_bytes") {
                        output.transparent_bytes = true;
                    } else if meta.path.is_ident("transparent_map") {
                        output.transparent_map = true;
                    } else if meta.path.is_ident("unsigned_minimum") {
                        output.unsigned_minimum = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u64>()?,
                        );
                    } else if meta.path.is_ident("unsigned_maximum") {
                        output.unsigned_maximum = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u64>()?,
                        );
                    } else {
                        return Err(meta.error("unsupported #[mfm(...)] container attribute"));
                    }
                    Ok(())
                })?;
            } else if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename_all") {
                        let rename_all = meta.value()?.parse::<LitStr>()?.value();
                        match rename_all.as_str() {
                            "snake_case" | "kebab-case" | "camelCase" => {
                                output.rename_all = Some(rename_all);
                                Ok(())
                            }
                            _ => {
                                Err(meta
                                    .error("unsupported serde(rename_all) value for MFM derive"))
                            }
                        }
                    } else if meta.path.is_ident("tag") {
                        output.enum_tag = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("content") {
                        output.enum_content = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("try_from") || meta.path.is_ident("into") {
                        let _ = meta.value()?.parse::<LitStr>()?;
                        Ok(())
                    } else if meta.path.is_ident("bound") {
                        if meta.input.peek(syn::token::Paren) {
                            meta.parse_nested_meta(|nested| {
                                if nested.input.peek(syn::Token![=]) {
                                    let _ = nested.value()?.parse::<LitStr>()?;
                                }
                                Ok(())
                            })?;
                        } else {
                            let _ = meta.value()?.parse::<LitStr>()?;
                        }
                        Ok(())
                    } else if meta.path.is_ident("transparent") {
                        output.serde_transparent = true;
                        Ok(())
                    } else if meta.path.is_ident("deny_unknown_fields") {
                        Ok(())
                    } else if meta.path.is_ident("untagged") {
                        Err(meta.error("serde(untagged) is not supported by MFM derives"))
                    } else {
                        Err(meta
                            .error("unsupported #[serde(...)] container attribute for MFM derive"))
                    }
                })?;
            }
        }

        if [
            output.transparent_string,
            output.transparent_bytes,
            output.transparent_map,
        ]
        .into_iter()
        .filter(|selected| *selected)
        .count()
            > 1
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "MFM derives accept only one transparent container mode",
            ));
        }
        if output
            .unsigned_minimum
            .zip(output.unsigned_maximum)
            .is_none()
            && (output.unsigned_minimum.is_some() || output.unsigned_maximum.is_some())
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "unsigned bounds require both unsigned_minimum and unsigned_maximum",
            ));
        }
        if output
            .unsigned_minimum
            .zip(output.unsigned_maximum)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "unsigned_minimum cannot exceed unsigned_maximum",
            ));
        }

        Ok(output)
    }
}

#[derive(Default)]
pub(super) struct FieldAttrs {
    pub(super) rename: Option<String>,
    pub(super) default: bool,
    pub(super) persisted: bool,
    pub(super) literal: Option<String>,
    pub(super) optional_absent: bool,
    pub(super) minimum_items: Option<u32>,
    pub(super) maximum_items: Option<u32>,
    pub(super) minimum_bytes: Option<u32>,
    pub(super) maximum_bytes: Option<u32>,
    pub(super) unsigned_minimum: Option<u64>,
    pub(super) unsigned_maximum: Option<u64>,
}

impl FieldAttrs {
    pub(super) fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        for attr in attrs {
            if attr.path().is_ident("mfm") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("persisted") {
                        output.persisted = true;
                        Ok(())
                    } else if meta.path.is_ident("literal") {
                        output.literal = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("minimum_items") {
                        output.minimum_items = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u32>()?,
                        );
                        Ok(())
                    } else if meta.path.is_ident("maximum_items") {
                        output.maximum_items = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u32>()?,
                        );
                        Ok(())
                    } else if meta.path.is_ident("minimum_bytes") {
                        output.minimum_bytes = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u32>()?,
                        );
                        Ok(())
                    } else if meta.path.is_ident("maximum_bytes") {
                        output.maximum_bytes = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u32>()?,
                        );
                        Ok(())
                    } else if meta.path.is_ident("unsigned_minimum") {
                        output.unsigned_minimum = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u64>()?,
                        );
                        Ok(())
                    } else if meta.path.is_ident("unsigned_maximum") {
                        output.unsigned_maximum = Some(
                            meta.value()?
                                .parse::<syn::LitInt>()?
                                .base10_parse::<u64>()?,
                        );
                        Ok(())
                    } else {
                        Err(meta.error("unsupported #[mfm(...)] field attribute"))
                    }
                })?;
            } else if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("default") {
                        if meta.input.peek(syn::Token![=]) {
                            return Err(
                                meta.error("custom serde default functions are not supported")
                            );
                        }
                        output.default = true;
                        Ok(())
                    } else if meta.path.is_ident("skip_serializing_if") {
                        let predicate = meta.value()?.parse::<LitStr>()?.value();
                        if predicate != "Option::is_none" {
                            return Err(meta.error(
                                "only serde(skip_serializing_if = \"Option::is_none\") is supported",
                            ));
                        }
                        output.optional_absent = true;
                        Ok(())
                    } else if meta.path.is_ident("skip")
                        || meta.path.is_ident("skip_serializing")
                        || meta.path.is_ident("skip_deserializing")
                    {
                        Err(meta.error("skipped fields are not supported by MFM derives"))
                    } else if meta.path.is_ident("flatten") {
                        Err(meta.error("serde(flatten) is not supported by MFM derives"))
                    } else if meta.path.is_ident("serialize_with")
                        || meta.path.is_ident("deserialize_with")
                        || meta.path.is_ident("with")
                    {
                        Err(meta.error("custom serde serializers are not supported by MFM derives"))
                    } else {
                        Err(meta.error("unsupported #[serde(...)] field attribute for MFM derive"))
                    }
                })?;
            }
        }
        if (output.minimum_items.is_some() || output.maximum_items.is_some()) && !output.persisted {
            return Err(syn::Error::new(
                Span::call_site(),
                "sequence bounds require #[mfm(persisted, ...)]",
            ));
        }
        if output.minimum_bytes.zip(output.maximum_bytes).is_none()
            && (output.minimum_bytes.is_some() || output.maximum_bytes.is_some())
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "byte bounds require both minimum_bytes and maximum_bytes",
            ));
        }
        if output
            .unsigned_minimum
            .zip(output.unsigned_maximum)
            .is_none()
            && (output.unsigned_minimum.is_some() || output.unsigned_maximum.is_some())
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "unsigned bounds require both unsigned_minimum and unsigned_maximum",
            ));
        }
        if output.literal.is_some()
            && (output.minimum_items.is_some()
                || output.maximum_items.is_some()
                || output.minimum_bytes.is_some()
                || output.maximum_bytes.is_some()
                || output.unsigned_minimum.is_some()
                || output.unsigned_maximum.is_some()
                || output.default)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "literal fields cannot also declare defaults, sequence bounds, or byte bounds",
            ));
        }
        if (output.minimum_items.is_some() || output.maximum_items.is_some())
            && (output.minimum_bytes.is_some() || output.maximum_bytes.is_some())
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "a field cannot declare both sequence and byte bounds",
            ));
        }
        let selected_bounds = [
            output.minimum_items.is_some(),
            output.minimum_bytes.is_some(),
            output.unsigned_minimum.is_some(),
        ]
        .into_iter()
        .filter(|selected| *selected)
        .count();
        if selected_bounds > 1 {
            return Err(syn::Error::new(
                Span::call_site(),
                "a field cannot combine sequence, byte, and unsigned bounds",
            ));
        }
        if output
            .minimum_items
            .zip(output.maximum_items)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "minimum_items cannot exceed maximum_items",
            ));
        }
        if output
            .unsigned_minimum
            .zip(output.unsigned_maximum)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "unsigned_minimum cannot exceed unsigned_maximum",
            ));
        }
        if output
            .minimum_bytes
            .zip(output.maximum_bytes)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "minimum_bytes cannot exceed maximum_bytes",
            ));
        }
        Ok(output)
    }
}

#[derive(Debug, Default)]
pub(super) struct VariantAttrs {
    pub(super) rename: Option<String>,
}

impl VariantAttrs {
    pub(super) fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        for attr in attrs {
            if attr.path().is_ident("serde") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        output.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else if meta.path.is_ident("alias") {
                        Err(meta.error("serde(alias) is not supported by MFM derives"))
                    } else {
                        Err(meta
                            .error("unsupported #[serde(...)] variant attribute for MFM derive"))
                    }
                })?;
            }
        }
        Ok(output)
    }
}
