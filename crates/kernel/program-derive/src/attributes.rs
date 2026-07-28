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
    pub(super) validate: Option<Path>,
    pub(super) transparent_string: bool,
    pub(super) transparent_map: bool,
    pub(super) serde_transparent: bool,
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
            validate: None,
            transparent_string: false,
            transparent_map: false,
            serde_transparent: false,
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
                    } else if meta.path.is_ident("validate") {
                        let value = meta.value()?.parse::<LitStr>()?.value();
                        output.validate =
                            Some(syn::parse_str::<Path>(&value).map_err(|error| {
                                syn::Error::new(
                                    meta.path.span(),
                                    format!("invalid config validator path: {error}"),
                                )
                            })?);
                    } else if meta.path.is_ident("transparent_string") {
                        output.transparent_string = true;
                    } else if meta.path.is_ident("transparent_map") {
                        output.transparent_map = true;
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

        if output.transparent_string && output.transparent_map {
            return Err(syn::Error::new(
                Span::call_site(),
                "MFM derives accept only one transparent container mode",
            ));
        }

        Ok(output)
    }
}

#[derive(Default)]
pub(super) struct FieldAttrs {
    pub(super) rename: Option<String>,
    pub(super) default: bool,
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
