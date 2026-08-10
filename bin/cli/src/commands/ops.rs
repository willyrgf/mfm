use clap::Subcommand;
use mfm_app::{PublicError, PublishedEntryPoint};

use crate::commands::CommandContext;
use crate::presentation::output::handle_public_result;
use crate::support::application::{connect_application, ApplicationConnectionArgs};

/// Public entry-point discovery commands.
#[derive(Subcommand)]
pub(crate) enum OpsCommand {
    /// List every complete compiled entry-point contract.
    List {
        /// Non-semantic process connection options.
        #[command(flatten)]
        connection: ApplicationConnectionArgs,
    },
}

impl OpsCommand {
    /// Dispatches the selected discovery command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        match self {
            Self::List { connection } => {
                let result = connect_application(connection)
                    .await
                    .map(|application| application.entry_points().to_vec());
                handle_public_result(result, &ctx.output_format, |entries| {
                    entry_points_text(entries)
                })
            }
        }
    }
}

fn entry_points_text(entries: &[PublishedEntryPoint]) -> Result<String, PublicError> {
    let mut text = entries
        .iter()
        .map(|entry| entry.entry_point_id().as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use mfm_app::{PlanningProfile, PublicJsonResponse, PublishedEntryPoint};
    use mfm_ids::{EntryPointId, SchemaId, StableId};

    use super::entry_points_text;
    use crate::commands::OutputFormat;
    use crate::presentation::output::render_public_result;

    #[test]
    fn entry_point_list_renders_complete_reviewed_contracts_and_ids() {
        let entries = vec![
            entry_point("mfm.test/first@1"),
            entry_point("mfm.test/second@2"),
        ];

        assert_eq!(
            render_public_result(&entries, &OutputFormat::Text, |entries| {
                entry_points_text(entries)
            })
            .expect("entry-point text"),
            "mfm.test/first@1\nmfm.test/second@2\n"
        );
        let expected = serde_json::to_string_pretty(
            &entries.public_json().expect("reviewed entry-point JSON"),
        )
        .expect("pretty entry-point JSON");
        assert_eq!(
            render_public_result(&entries, &OutputFormat::Json, |entries| {
                entry_points_text(entries)
            })
            .expect("entry-point JSON"),
            format!("{expected}\n")
        );
    }

    fn entry_point(id: &str) -> PublishedEntryPoint {
        let profile = PlanningProfile::from_canonical_json(
            br#"{"canonical_profile_parameters":{},"framework_policy_refs":[],"planner_contract_ref":{"content_digest":"content:sha256-v1:1111111111111111111111111111111111111111111111111111111111111111","schema_id":"schema:mfm.test.component:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"planner_implementation_ref":{"content_digest":"content:sha256-v1:2222222222222222222222222222222222222222222222222222222222222222","schema_id":"schema:mfm.test.component:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"version":"mfm.planning-profile.v1"}"#,
        )
        .expect("planning profile");
        PublishedEntryPoint::new(
            EntryPointId::new(id).expect("entry-point id"),
            StableId::new("mfm.test/operation").expect("operation id"),
            profile,
            schema_id("mfm.test.input"),
            schema_id("mfm.test.output"),
        )
        .expect("entry-point contract")
    }

    fn schema_id(name: &str) -> SchemaId {
        SchemaId::parse(format!(
            "schema:{name}:1:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333"
        ))
        .expect("schema id")
    }
}
