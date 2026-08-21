use percent_encoding::percent_decode_str;
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::ConnectOptions;
use url::Url;

/// Maximum encoded bytes accepted in one private PostgreSQL locator.
pub const MAX_POSTGRES_LOCATOR_BYTES: usize = 16 * 1024;

const RUNTIME_ROLE: &str = "mfm_runtime";
const BLOCKED_POSTGRES_ENVIRONMENT: &str = "PGOPTIONS";

/// Redaction-safe private PostgreSQL locator failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("postgres locator is invalid or unavailable")]
pub struct PostgresLocatorError;

/// Checked runtime-role PostgreSQL connection authority.
///
/// This value deliberately implements neither `Debug`, `Display`, nor serialization.
pub struct RuntimePostgresLocator(PostgresLocator);

impl RuntimePostgresLocator {
    /// Parses one bounded locator and requires the fixed `mfm_runtime` role.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PostgresLocatorError> {
        let locator = PostgresLocator::parse(value.as_ref())?;
        if locator.username != RUNTIME_ROLE {
            return Err(PostgresLocatorError);
        }
        Ok(Self(locator))
    }

    pub(super) fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<PgConnectOptions, PostgresLocatorError> {
        self.0.connect_options(application_name)
    }

    pub(super) const fn target(&self) -> &PostgresTarget {
        &self.0.target
    }
}

/// Checked administrative PostgreSQL connection authority.
///
/// This value deliberately implements neither `Debug`, `Display`, nor serialization.
pub struct AdminPostgresLocator(PostgresLocator);

impl AdminPostgresLocator {
    /// Parses one bounded locator and rejects the fixed runtime role.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PostgresLocatorError> {
        let locator = PostgresLocator::parse(value.as_ref())?;
        if locator.username == RUNTIME_ROLE {
            return Err(PostgresLocatorError);
        }
        Ok(Self(locator))
    }

    pub(super) fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<PgConnectOptions, PostgresLocatorError> {
        self.0.connect_options(application_name)
    }

    pub(super) const fn target(&self) -> &PostgresTarget {
        &self.0.target
    }
}

struct PostgresLocator {
    target: PostgresTarget,
    username: String,
    password: String,
}

#[derive(PartialEq, Eq)]
pub(super) struct PostgresTarget {
    host: String,
    port: u16,
    database: String,
}

impl PostgresLocator {
    fn parse(value: &str) -> Result<Self, PostgresLocatorError> {
        if value.is_empty() || value.len() > MAX_POSTGRES_LOCATOR_BYTES {
            return Err(PostgresLocatorError);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            v: u8,
            url: String,
        }
        let wire: Wire = serde_json::from_str(value).map_err(|_| PostgresLocatorError)?;
        if wire.v != 1 || wire.url.is_empty() || wire.url.len() > MAX_POSTGRES_LOCATOR_BYTES {
            return Err(PostgresLocatorError);
        }
        verify_raw_uri_shape(&wire.url)?;
        let url = Url::parse(&wire.url).map_err(|_| PostgresLocatorError)?;
        if url.scheme() != "postgresql"
            || url.cannot_be_a_base()
            || url.fragment().is_some()
            || !url.has_host()
        {
            return Err(PostgresLocatorError);
        }
        let host = normalized_host(&url)?;
        let username = decode_component(url.username(), 63)?;
        let password = decode_component(url.password().ok_or(PostgresLocatorError)?, 4096)?;
        let database = decode_database(url.path())?;
        let query = url.query_pairs().collect::<Vec<_>>();
        if query.len() != 1 || query[0].0 != "sslmode" || query[0].1 != "disable" {
            return Err(PostgresLocatorError);
        }
        Ok(Self {
            target: PostgresTarget {
                host,
                port: url.port().unwrap_or(5432),
                database,
            },
            username,
            password,
        })
    }

    fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<PgConnectOptions, PostgresLocatorError> {
        if std::env::var_os(BLOCKED_POSTGRES_ENVIRONMENT).is_some() {
            return Err(PostgresLocatorError);
        }
        let options = PgConnectOptions::new_without_pgpass()
            .host(&self.target.host)
            .port(self.target.port)
            .username(&self.username)
            .password(&self.password)
            .database(&self.target.database)
            .ssl_mode(PgSslMode::Disable)
            .statement_cache_capacity(100)
            .application_name(application_name)
            .disable_statement_logging();
        Ok(options)
    }
}

fn verify_raw_uri_shape(value: &str) -> Result<(), PostgresLocatorError> {
    if value.chars().any(char::is_control) {
        return Err(PostgresLocatorError);
    }
    let authority_and_path = value
        .strip_prefix("postgresql://")
        .and_then(|value| value.strip_suffix("?sslmode=disable"))
        .ok_or(PostgresLocatorError)?;
    if authority_and_path.contains(['?', '#']) {
        return Err(PostgresLocatorError);
    }
    let (_, database) = authority_and_path
        .split_once('/')
        .ok_or(PostgresLocatorError)?;
    if database.is_empty() || database.contains('/') {
        return Err(PostgresLocatorError);
    }
    Ok(())
}

fn normalized_host(url: &Url) -> Result<String, PostgresLocatorError> {
    let encoded = url.host_str().ok_or(PostgresLocatorError)?;
    let candidate = encoded
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(encoded);
    match candidate
        .parse::<std::net::IpAddr>()
        .map_err(|_| PostgresLocatorError)?
    {
        std::net::IpAddr::V4(value) if value == std::net::Ipv4Addr::LOCALHOST => {
            Ok(value.to_string())
        }
        std::net::IpAddr::V6(value) if value == std::net::Ipv6Addr::LOCALHOST => {
            Ok(value.to_string())
        }
        std::net::IpAddr::V4(_) | std::net::IpAddr::V6(_) => Err(PostgresLocatorError),
    }
}

fn decode_component(value: &str, maximum: usize) -> Result<String, PostgresLocatorError> {
    let decoded = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| PostgresLocatorError)?;
    if decoded.is_empty()
        || decoded.len() > maximum
        || decoded.as_bytes().contains(&0)
        || decoded.chars().any(char::is_control)
    {
        return Err(PostgresLocatorError);
    }
    Ok(decoded.into_owned())
}

fn decode_database(path: &str) -> Result<String, PostgresLocatorError> {
    let encoded = path.strip_prefix('/').ok_or(PostgresLocatorError)?;
    let database = decode_component(encoded, 63)?;
    if database.contains('/') {
        return Err(PostgresLocatorError);
    }
    Ok(database)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locator(url: &str) -> String {
        format!(r#"{{"v":1,"url":{url:?}}}"#)
    }

    fn authority(byte: char) -> String {
        byte.to_string().repeat(32)
    }

    fn strict_url(role: &str, authority: &str, host: &str, database: &str) -> String {
        format!("postgresql://{role}:{authority}@{host}/{database}?sslmode=disable")
    }

    #[test]
    fn private_locator_grammar_is_narrow_and_role_typed() {
        let left = authority('a');
        let right = authority('b');
        let encoded_authority = format!("{left}%40{right}");
        let runtime_url = strict_url("mfm_runtime", &encoded_authority, "127.0.0.1:5433", "mfm");
        let runtime = locator(&runtime_url);
        let runtime = RuntimePostgresLocator::parse(&runtime).expect("runtime locator");
        assert_eq!(runtime.0.username, "mfm_runtime");
        assert_eq!(runtime.0.password, format!("{left}@{right}"));
        assert_eq!(runtime.0.target.host, "127.0.0.1");
        assert_eq!(runtime.0.target.port, 5433);
        assert_eq!(runtime.0.target.database, "mfm");
        assert!(AdminPostgresLocator::parse(locator(&strict_url(
            "admin",
            &authority('c'),
            "[::1]",
            "mfm",
        )))
        .is_ok());
        assert!(AdminPostgresLocator::parse(locator(&strict_url(
            "mfm_runtime",
            &authority('d'),
            "127.0.0.1",
            "mfm",
        )))
        .is_err());
    }

    #[test]
    fn private_locator_rejects_ambient_or_ambiguous_authority_forms() {
        let authority = authority('e');
        let valid = strict_url("mfm_runtime", &authority, "127.0.0.1", "mfm");
        for url in [
            valid.trim_end_matches("?sslmode=disable").to_owned(),
            valid.replace("disable", "require"),
            valid.replace("disable", "verify-full"),
            format!("{valid}&application_name=x"),
            valid.replace("sslmode", "%73slmode"),
            valid.replace("/mfm?", "/a/../mfm?"),
            "postgresql://mfm_runtime@127.0.0.1/mfm?sslmode=disable".to_owned(),
            "postgresql://mfm_runtime:@127.0.0.1/mfm?sslmode=disable".to_owned(),
            format!("postgresql://:{authority}@127.0.0.1/mfm?sslmode=disable"),
            valid.replace("/mfm?", "/?"),
            valid.replace("/mfm?", "/a/b?"),
            format!("{valid}#fragment"),
            valid.replacen("postgresql", "postgres", 1),
            valid.replace("127.0.0.1", "localhost"),
            valid.replace("127.0.0.1", "192.0.2.1"),
            valid.replace("127.0.0.1", "127.0.0.2"),
            valid.replace("127.0.0.1", "[::2]"),
        ] {
            assert!(RuntimePostgresLocator::parse(locator(&url)).is_err());
        }
        for rejected in [
            format!(r#"{{"v":1,"url":{valid:?},"extra":true}}"#),
            format!(r#"{{"v":1,"url":{valid:?},"tls_roots":{{"kind":"webpki"}}}}"#),
            format!(r#"{{"v":1,"url":{valid:?},"tls_trust":{{"kind":"webpki"}}}}"#),
        ] {
            assert!(RuntimePostgresLocator::parse(rejected).is_err());
        }
    }

    #[test]
    fn target_equivalence_ignores_only_credentials() {
        let admin = AdminPostgresLocator::parse(locator(&strict_url(
            "admin",
            &authority('f'),
            "127.0.0.1",
            "mfm",
        )))
        .expect("admin");
        let runtime = RuntimePostgresLocator::parse(locator(&strict_url(
            "mfm_runtime",
            &authority('0'),
            "127.0.0.1:5432",
            "mfm",
        )))
        .expect("runtime");
        assert!(admin.target() == runtime.target());
        let other = RuntimePostgresLocator::parse(locator(&strict_url(
            "mfm_runtime",
            &authority('1'),
            "127.0.0.1:5432",
            "other",
        )))
        .expect("other");
        assert!(admin.target() != other.target());
    }
}
