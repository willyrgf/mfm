use mfm_transport_security::{LoadedTlsRoots, TlsRootSpec};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::ConnectOptions;
use url::{Host, Url};

/// Maximum encoded bytes accepted in one private PostgreSQL locator.
pub const MAX_POSTGRES_LOCATOR_BYTES: usize = 16 * 1024;

const RUNTIME_ROLE: &str = "mfm_runtime";

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

    pub(super) async fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<(PgConnectOptions, LoadedTlsRoots), PostgresLocatorError> {
        self.0.connect_options(application_name).await
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

    pub(super) async fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<(PgConnectOptions, LoadedTlsRoots), PostgresLocatorError> {
        self.0.connect_options(application_name).await
    }

    pub(super) const fn target(&self) -> &PostgresTarget {
        &self.0.target
    }
}

struct PostgresLocator {
    target: PostgresTarget,
    username: String,
    password: String,
    tls_roots: TlsRootSpec,
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
            tls_roots: TlsRootSpec,
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
        if query.len() != 1 || query[0].0 != "sslmode" || query[0].1 != "verify-full" {
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
            tls_roots: wire.tls_roots,
        })
    }

    async fn connect_options(
        &self,
        application_name: &'static str,
    ) -> Result<(PgConnectOptions, LoadedTlsRoots), PostgresLocatorError> {
        let roots = LoadedTlsRoots::load(&self.tls_roots)
            .await
            .map_err(|_| PostgresLocatorError)?;
        let options = PgConnectOptions::new_without_env_or_files()
            .host(&self.target.host)
            .port(self.target.port)
            .username(&self.username)
            .password(&self.password)
            .database(&self.target.database)
            .ssl_mode(PgSslMode::VerifyFull)
            .ssl_root_cert_store(roots.root_store())
            .statement_cache_capacity(100)
            .application_name(application_name)
            .disable_statement_logging();
        Ok((options, roots))
    }
}

fn verify_raw_uri_shape(value: &str) -> Result<(), PostgresLocatorError> {
    if value.chars().any(char::is_control) {
        return Err(PostgresLocatorError);
    }
    let authority_and_path = value
        .strip_prefix("postgresql://")
        .and_then(|value| value.strip_suffix("?sslmode=verify-full"))
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
    match url.host().ok_or(PostgresLocatorError)? {
        Host::Domain(value) => {
            if value.is_empty() || value.len() > 253 {
                return Err(PostgresLocatorError);
            }
            Ok(value.to_ascii_lowercase())
        }
        Host::Ipv4(value) => Ok(value.to_string()),
        Host::Ipv6(value) => Ok(value.to_string()),
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

    fn locator(url: &str, roots: &str) -> String {
        format!(r#"{{"v":1,"url":{url:?},"tls_roots":{roots}}}"#)
    }

    fn authority(byte: char) -> String {
        byte.to_string().repeat(32)
    }

    fn strict_url(role: &str, authority: &str, host: &str, database: &str) -> String {
        format!("postgresql://{role}:{authority}@{host}/{database}?sslmode=verify-full")
    }

    #[test]
    fn private_locator_grammar_is_narrow_and_role_typed() {
        let left = authority('a');
        let right = authority('b');
        let encoded_authority = format!("{left}%40{right}");
        let runtime = locator(
            &strict_url("mfm_runtime", &encoded_authority, "db.example:5433", "mfm"),
            r#"{"kind":"webpki"}"#,
        );
        let runtime = RuntimePostgresLocator::parse(&runtime).expect("runtime locator");
        assert_eq!(runtime.0.username, "mfm_runtime");
        assert_eq!(runtime.0.password, format!("{left}@{right}"));
        assert_eq!(runtime.0.target.host, "db.example");
        assert_eq!(runtime.0.target.port, 5433);
        assert_eq!(runtime.0.target.database, "mfm");
        assert!(AdminPostgresLocator::parse(locator(
            &strict_url("admin", &authority('c'), "db.example", "mfm"),
            r#"{"kind":"webpki"}"#,
        ))
        .is_ok());
        assert!(AdminPostgresLocator::parse(locator(
            &strict_url("mfm_runtime", &authority('d'), "db.example", "mfm"),
            r#"{"kind":"webpki"}"#,
        ))
        .is_err());
    }

    #[test]
    fn private_locator_rejects_ambient_or_ambiguous_authority_forms() {
        let authority = authority('e');
        let valid = strict_url("mfm_runtime", &authority, "db.example", "mfm");
        for url in [
            valid.trim_end_matches("?sslmode=verify-full").to_owned(),
            valid.replace("verify-full", "require"),
            format!("{valid}&application_name=x"),
            valid.replace("sslmode", "%73slmode"),
            valid.replace("/mfm?", "/a/../mfm?"),
            "postgresql://mfm_runtime@db.example/mfm?sslmode=verify-full".to_owned(),
            "postgresql://mfm_runtime:@db.example/mfm?sslmode=verify-full".to_owned(),
            format!("postgresql://:{authority}@db.example/mfm?sslmode=verify-full"),
            valid.replace("/mfm?", "/?"),
            valid.replace("/mfm?", "/a/b?"),
            format!("{valid}#fragment"),
            valid.replacen("postgresql", "postgres", 1),
        ] {
            assert!(RuntimePostgresLocator::parse(locator(&url, r#"{"kind":"webpki"}"#)).is_err());
        }
        let extra =
            format!(r#"{{"v":1,"url":{valid:?},"tls_roots":{{"kind":"webpki"}},"extra":true}}"#);
        assert!(RuntimePostgresLocator::parse(extra).is_err());
    }

    #[test]
    fn target_equivalence_ignores_only_credentials_and_security_roots() {
        let admin = AdminPostgresLocator::parse(locator(
            &strict_url("admin", &authority('f'), "DB.EXAMPLE", "mfm"),
            r#"{"kind":"webpki"}"#,
        ))
        .expect("admin");
        let runtime = RuntimePostgresLocator::parse(locator(
            &strict_url("mfm_runtime", &authority('0'), "db.example:5432", "mfm"),
            r#"{"kind":"pem-file","path":"/tmp/ca.pem","digest":"content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ))
        .expect("runtime");
        assert!(admin.target() == runtime.target());
        let other = RuntimePostgresLocator::parse(locator(
            &strict_url("mfm_runtime", &authority('1'), "db.example:5432", "other"),
            r#"{"kind":"webpki"}"#,
        ))
        .expect("other");
        assert!(admin.target() != other.target());
    }
}
