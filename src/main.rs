use std::io::Read;

use azure_core::auth::TokenCredential;
use azure_identity::DefaultAzureCredential;
use clap::{crate_name, Parser};
use log::{info, trace};
use serde::Deserialize;
use serde_json::json;
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The operation this program was run with.
    operation: Operation,

    /// Logging level. Set to one of
    /// `error`, `warn`, `info`, `debug`, or `trace`.
    #[clap(long, env = "ACR_LOGIN_LOG_LEVEL", default_value = "warn")]
    log_level: log::Level,

    // Azure Tenant ID. Must be the tenant where the ACR is
    // located.
    #[clap(long, env = "AZURE_TENANT_ID")]
    azure_tenant_id: String,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum Operation {
    Store,
    Get,
    Erase,
    List,
}

// The standard ACR domain for Azure
const ACR_DOMAIN: &str = ".azurecr.io";

// The standard ACR username
const ACR_USERNAME: &str = "00000000-0000-0000-0000-000000000000";

// Result with boxed Result
type StdResult<T> = Result<T, Box<dyn std::error::Error>>;

// ACR refresh token structure
#[derive(Deserialize)]
struct AcrRefreshToken {
    refresh_token: String,
}

#[tokio::main]
async fn main() -> StdResult<()> {
    let cli = Cli::parse();

    // Set up the stderr logger
    stderrlog::new()
        .module(module_path!())
        .verbosity(cli.log_level)
        .init()?;

    match cli.operation {
        Operation::Get => get_docker_credential(&cli).await?,

        // For other operations, do nothing.
        _ => info!("{} is a read-only provider", crate_name!()),
    }

    Ok(())
}

async fn get_docker_credential(cli: &Cli) -> StdResult<()> {
    // Expecting the registry of the ACR as input from stdin.
    let mut registry = String::new();
    std::io::stdin().read_to_string(&mut registry)?;

    // Trim the registry of whitespace.
    let registry = registry.trim();

    // If the url doesn't end with ".azurecr.io" then
    // we don't know how to handle this.
    if !registry.ends_with(ACR_DOMAIN) {
        // Normal operation when used as a credStore.
        info!("{} not handling registry: {}", crate_name!(), registry);
        return Ok(());
    }

    // Need to connect to the repository's OAuth endpoint to exchange the token we just got.
    let url = Url::parse(&format!("https://{}/oauth2/exchange", registry))?;

    // Time to obtain some credentials and variables!
    let credential = DefaultAzureCredential::default();
    let token_response = credential.get_token("https://management.azure.com").await?;
    let tenant = &cli.azure_tenant_id;

    // Set up the parameters for the post to the OAuth endpoint as per
    // https://github.com/Azure/acr/blob/main/docs/AAD-OAuth.md#calling-post-oauth2exchange-to-get-an-acr-refresh-token
    let grant_type = String::from("access_token");
    let params = [
        ("grant_type", grant_type.as_str()),
        ("service", registry),
        ("tenant", tenant.as_str()),
        ("access_token", token_response.token.secret()),
    ];
    trace!("Params: {:?}", params);

    // Send the request to the endpoint in order to get the ACR refresh token.
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json::<AcrRefreshToken>()
        .await?;

    // At this point we have a refresh token, which can be provided to
    // Docker.
    let creds = json!({
        "Username": ACR_USERNAME,
        "Secret": response.refresh_token
    });
    trace!("Credentials: {:?}", creds);

    // Write the credentials to stdout
    serde_json::to_writer(std::io::stdout(), &creds)?;

    // If we get here we succeeded!
    Ok(())
}
