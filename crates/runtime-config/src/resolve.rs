use super::*;
use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};

pub(super) fn parse_source_ref(raw: &str, location: RuntimeConfigLocation) -> Result<EvmSourceRef> {
    EvmSourceRef::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::SourceRef,
            },
        )
    })
}

pub(super) fn parse_policy_id(
    raw: &str,
    location: RuntimeConfigLocation,
) -> Result<EvmSourcePolicyId> {
    EvmSourcePolicyId::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::PolicyId,
            },
        )
    })
}

pub(super) fn resolve_required_value(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<RuntimeSecretValue> {
    let location = location.with_field(field);
    let source = select_value_source(
        location.clone(),
        true,
        direct,
        env_name,
        file_path,
        file_env,
    )?
    .expect("required value source returns Some");
    resolve_selected_value(location, source)
}

pub(super) fn resolve_rpc_url(
    location: RuntimeConfigLocation,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<RuntimeSecretValue> {
    let rpc_url = resolve_required_value(
        location.clone(),
        "rpc_url",
        direct,
        env_name,
        file_path,
        file_env,
    )?;
    validate_rpc_url(&rpc_url, location.with_field("rpc_url"))?;
    Ok(rpc_url)
}

pub(super) fn resolve_optional_value(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<Option<RuntimeSecretValue>> {
    let location = location.with_field(field);
    let Some(source) = select_value_source(
        location.clone(),
        false,
        direct,
        env_name,
        file_path,
        file_env,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(resolve_selected_value(location, source)?))
}

pub(super) fn resolve_required_path(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<RuntimeSecretPath> {
    let value = resolve_required_value(location, field, direct, env_name, file_path, file_env)?;
    Ok(RuntimeSecretPath::from_value(value))
}

enum SelectedValueSource<'a> {
    Direct(&'a str),
    Env(&'a str),
    File(&'a str),
    FileEnv(&'a str),
}

impl SelectedValueSource<'_> {
    const fn kind(&self) -> RuntimeValueSourceKind {
        match self {
            Self::Direct(_) => RuntimeValueSourceKind::Direct,
            Self::Env(_) => RuntimeValueSourceKind::Env,
            Self::File(_) => RuntimeValueSourceKind::File,
            Self::FileEnv(_) => RuntimeValueSourceKind::FileEnv,
        }
    }
}

fn select_value_source<'a>(
    location: RuntimeConfigLocation,
    required: bool,
    direct: &'a Option<String>,
    env_name: &'a Option<String>,
    file_path: &'a Option<String>,
    file_env: &'a Option<String>,
) -> Result<Option<SelectedValueSource<'a>>> {
    let mut selected = Vec::new();
    if let Some(value) = direct.as_deref() {
        selected.push(SelectedValueSource::Direct(value));
    }
    if let Some(value) = env_name.as_deref() {
        selected.push(SelectedValueSource::Env(value));
    }
    if let Some(value) = file_path.as_deref() {
        selected.push(SelectedValueSource::File(value));
    }
    if let Some(value) = file_env.as_deref() {
        selected.push(SelectedValueSource::FileEnv(value));
    }

    match selected.len() {
        0 if required => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        )),
        0 => Ok(None),
        1 => Ok(selected.into_iter().next()),
        _ if required => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        )),
        _ => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::AtMostOneValueSource,
        )),
    }
}

fn resolve_selected_value(
    location: RuntimeConfigLocation,
    source: SelectedValueSource<'_>,
) -> Result<RuntimeSecretValue> {
    let source_kind = source.kind();
    let value = match source {
        SelectedValueSource::Direct(value) => value.to_owned(),
        SelectedValueSource::Env(name) => {
            let name = parse_env_name(name, location.clone())?;
            read_env_value(name.as_str(), location.clone())?
        }
        SelectedValueSource::File(path) => read_indirection_file(path, location.clone())?,
        SelectedValueSource::FileEnv(name) => {
            let name = parse_env_name(name, location.clone())?;
            let path = read_env_value(name.as_str(), location.clone())?;
            read_indirection_file(path.trim(), location.clone())?
        }
    };
    let value = match source_kind {
        RuntimeValueSourceKind::File | RuntimeValueSourceKind::FileEnv => value.trim().to_owned(),
        RuntimeValueSourceKind::Direct | RuntimeValueSourceKind::Env => value,
    };
    if value.trim().is_empty() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    Ok(RuntimeSecretValue::from_parts(value, source_kind))
}

fn parse_env_name(name: &str, location: RuntimeConfigLocation) -> Result<RuntimeEnvName> {
    RuntimeEnvName::new(name).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::EnvName,
            },
        )
    })
}

fn read_env_value(name: &str, location: RuntimeConfigLocation) -> Result<String> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::MissingEnv,
        )),
        Err(env::VarError::NotUnicode(_)) => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidEnv,
        )),
    }
}

fn read_indirection_file(path: &str, location: RuntimeConfigLocation) -> Result<String> {
    if path.trim().is_empty() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    fs::read_to_string(path)
        .map_err(|_| RuntimeConfigError::new(location, RuntimeConfigErrorKind::IndirectionFileRead))
}

pub(super) fn validate_rpc_url(
    value: &RuntimeSecretValue,
    location: RuntimeConfigLocation,
) -> Result<()> {
    let url = url::Url::parse(value.expose_secret()).map_err(|_| {
        RuntimeConfigError::new(location.clone(), RuntimeConfigErrorKind::InvalidUrl)
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::UrlUserInfo,
        ));
    }
    Ok(())
}
