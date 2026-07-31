use std::{io::Read, process::Command, sync::Arc};

use azure_core::credentials::{Secret, TokenCredential};
use azure_identity::{AzureCliCredential, AzureCliCredentialOptions};
use clap::{Parser, crate_name};
use log::{info, trace, warn};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;
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

    // Azure Tenant ID. Optional: it only decides which tenant is searched
    // first when locating the registry, saving a lookup in the common case.
    #[clap(long, env = "AZURE_TENANT_ID")]
    azure_tenant_id: Option<String>,
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

// How many subscriptions to query at once. Each one spawns an `az` process, so
// this bounds the load an account with many subscriptions can generate.
const MAX_CONCURRENT_QUERIES: usize = 10;

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
        // For gets, get any credentials for the given registry.
        Operation::Get => {
            let creds = get_docker_credential(&cli).await?;

            // Write the output to stdout
            serde_json::to_writer(std::io::stdout(), &creds)?;
        }

        // For other operations, do nothing.
        _ => info!("{} is a read-only provider", crate_name!()),
    }

    Ok(())
}

async fn get_docker_credential(cli: &Cli) -> StdResult<serde_json::Value> {
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

        // Return an empty JSON dictionary
        return Ok(json!({}));
    }

    let tenants = subscriptions_by_tenant(cli.azure_tenant_id.as_deref());

    // Prefer asking Resource Graph which subscription owns the registry: it
    // gets the answer in one step and avoids failed exchanges.
    let refresh_token = match locate_registry(registry, &tenants).await {
        Ok(location) => {
            info!(
                "Registry {registry} is in subscription {} (tenant {})",
                location.subscription_id, location.tenant_id
            );
            let token = token_for_subscription(&location.subscription_id).await?;
            exchange_token(registry, &location.tenant_id, &token).await?
        }

        // The lookup needs `Reader` on the registry, but pulling only needs
        // `AcrPull` - the exchange endpoint doesn't check ARM permissions. So a
        // failed lookup is not fatal: fall back to trying the exchange against
        // each tenant, which is all the lookup would have told us anyway.
        Err(e) => {
            info!("Could not look up {registry} ({e}); trying each tenant instead");
            exchange_with_any_tenant(registry, &tenants).await?
        }
    };

    let creds = json!({
        "Username": ACR_USERNAME,
        "Secret": refresh_token
    });
    trace!("Credentials: {creds:?}");

    Ok(creds)
}

/// Try the token exchange against each tenant until one is accepted.
///
/// An ACR only accepts a token issued by the tenant it lives in, so a rejected
/// exchange means "wrong tenant" and we move on.
async fn exchange_with_any_tenant(
    registry: &str,
    tenants: &[(String, Vec<String>)],
) -> StdResult<String> {
    // Subscriptions in one tenant can belong to different signed-in identities,
    // and only some may hold AcrPull, so every subscription is a candidate.
    // Attempt them concurrently: each is a token request plus an exchange, and
    // only one can succeed - the ACR rejects tokens from any other tenant.
    let limit = Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES));
    let attempts: Vec<_> = tenants
        .iter()
        .flat_map(|(tenant, subscriptions)| {
            subscriptions.iter().map(move |sub| (tenant.clone(), sub))
        })
        .map(|(tenant, subscription)| {
            let (registry, subscription) = (registry.to_string(), subscription.to_string());
            let limit = limit.clone();

            tokio::spawn(async move {
                let _permit = limit.acquire().await;
                info!("Trying subscription {subscription} (tenant {tenant}) for {registry}");

                // Flatten each error to a string as it happens: `Box<dyn Error>`
                // is not `Send`, so it cannot be held across the next await.
                let flatten =
                    |e: Box<dyn std::error::Error>| first_line(&e.to_string()).to_string();

                let result = match token_for_subscription(&subscription).await.map_err(flatten) {
                    Ok(token) => exchange_token(&registry, &tenant, &token)
                        .await
                        .map_err(flatten),
                    Err(e) => Err(e),
                };

                (subscription, result)
            })
        })
        .collect();

    let mut failures = Vec::new();
    for attempt in attempts {
        match attempt.await? {
            (_, Ok(refresh_token)) => return Ok(refresh_token),
            (subscription, Err(e)) => {
                info!("Subscription {subscription} did not work for {registry}: {e}");
                failures.push(format!("  {subscription}: {e}"));
            }
        }
    }

    Err(format!(
        "no Azure tenant could authenticate to {registry}. Tried:\n{}\n\
         You may need `az login --tenant <id>` for the tenant that owns it.",
        failures.join("\n")
    )
    .into())
}

/// Where a registry lives. The subscription is the important half: `az` can be
/// logged in as several identities at once, and only the subscription pins down
/// which one to use. A tenant alone is ambiguous and picks the active account's
/// identity, which may be a stranger in that tenant.
#[derive(Debug)]
struct RegistryLocation {
    subscription_id: String,
    tenant_id: String,
}

#[derive(Deserialize)]
struct GraphRow {
    #[serde(rename = "subscriptionId")]
    subscription_id: String,
    #[serde(rename = "tenantId")]
    tenant_id: String,
}

#[derive(Deserialize)]
struct GraphResponse {
    data: Vec<GraphRow>,
}

/// Ask Azure Resource Graph which subscription hosts `registry`.
///
/// A Resource Graph query covers one tenant at a time, so this runs once per
/// tenant the CLI is signed into until the registry turns up.
///
/// Note that Resource Graph only returns resources the identity can read, so a
/// registry you can pull from but lack `Reader` on simply isn't in the results.
/// "Not found" therefore means "not found or not visible", and callers should
/// treat it as inconclusive rather than as proof of absence.
async fn locate_registry(
    registry: &str,
    tenants: &[(String, Vec<String>)],
) -> StdResult<RegistryLocation> {
    // Match on the login server rather than the name: registry names are only
    // unique within a tenant, but `<name>.azurecr.io` is globally unique.
    let query = format!(
        "resources | where type =~ 'microsoft.containerregistry/registries' \
         | where tostring(properties.loginServer) =~ '{registry}' \
         | project subscriptionId, tenantId"
    );

    // Search subscriptions concurrently. The queries are read-only and a
    // registry lives in exactly one of them, so there is nothing to serialise:
    // done sequentially this is one `az` invocation plus one request per
    // subscription, which adds up quickly across a few tenants.
    let client = reqwest::Client::new();
    let limit = Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES));
    let searches: Vec<_> = tenants
        .iter()
        .flat_map(|(_, subscriptions)| subscriptions)
        .map(|subscription| {
            let (client, query, subscription) =
                (client.clone(), query.clone(), subscription.clone());
            let limit = limit.clone();

            tokio::spawn(async move {
                let _permit = limit.acquire().await;

                // Flatten the error to a string: `Box<dyn Error>` is not `Send`,
                // and only the message survives this boundary anyway.
                let result = query_subscription(&client, &query, &subscription)
                    .await
                    .map_err(|e| first_line(&e.to_string()).to_string());

                (subscription, result)
            })
        })
        .collect();

    let mut failures = Vec::new();
    for search in searches {
        match search.await? {
            (_, Ok(Some(location))) => return Ok(location),
            (subscription, Ok(None)) => {
                trace!("Subscription {subscription} does not hold {registry}");
            }
            (subscription, Err(e)) => {
                trace!("Subscription {subscription} could not be queried: {e}");
                failures.push(format!("{subscription}: {e}"));
            }
        }
    }

    // Report only the subscriptions that could not be searched: those are the
    // reason a "not found" result might be wrong.
    Err(match failures.is_empty() {
        true => "registry not found in any subscription".into(),
        false => format!("could not search {}", failures.join("; ")).into(),
    })
}

/// Run a Resource Graph query against one subscription, as that subscription's
/// own identity.
///
/// Scoping the query to a single subscription is deliberate: subscriptions in a
/// tenant may belong to different signed-in identities, and graph only returns
/// what the querying identity can read. Asking one identity about another's
/// subscription just returns nothing, silently.
async fn query_subscription(
    client: &reqwest::Client,
    query: &str,
    subscription: &str,
) -> StdResult<Option<RegistryLocation>> {
    let token = token_for_subscription(subscription).await?;

    let response = client
        .post("https://management.azure.com/providers/Microsoft.ResourceGraph/resources?api-version=2021-03-01")
        .bearer_auth(token.secret())
        .json(&json!({ "subscriptions": [subscription], "query": query }))
        .send()
        .await?
        .error_for_status()?
        .json::<GraphResponse>()
        .await?;

    Ok(response
        .data
        .into_iter()
        .next()
        .map(|row| RegistryLocation {
            subscription_id: row.subscription_id,
            tenant_id: row.tenant_id,
        }))
}

/// Get an ARM token as the identity that owns `subscription`.
///
/// Authenticating by subscription rather than tenant is deliberate: with several
/// identities signed in, a tenant alone resolves to the active account, which
/// may be a stranger in the tenant being queried.
async fn token_for_subscription(subscription: &str) -> StdResult<Secret> {
    let credential = AzureCliCredential::new(Some(AzureCliCredentialOptions {
        subscription: Some(subscription.to_string()),
        ..Default::default()
    }))?;

    let response = credential
        .get_token(&["https://management.azure.com/.default"], None)
        .await?;

    Ok(response.token)
}

/// Azure errors are multi-line essays; the first line carries the reason.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s).trim()
}

/// The CLI's subscriptions grouped by tenant, preferred tenant first.
fn subscriptions_by_tenant(preferred: Option<&str>) -> Vec<(String, Vec<String>)> {
    // Shell out to `az` to get the list of subscriptions and their tenants.
    // We're already expecting `az` to exist because we're using the
    // Azure CLI credential.
    let output = match Command::new("az")
        .args([
            "account",
            "list",
            "--query",
            "[].[tenantId,id]",
            "-o",
            "tsv",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output.stdout,
        // Without the account list there are no tenants to search, so the
        // lookup and every fallback are guaranteed to fail. Warn rather than
        // inform: this is the actual cause of whatever error follows.
        Ok(output) => {
            warn!(
                "az account list failed, cannot find any tenants: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return Vec::new();
        }
        Err(e) => {
            warn!("could not run az, cannot find any tenants: {e}");
            return Vec::new();
        }
    };

    let mut tenants: Vec<(String, Vec<String>)> = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let Some((tenant, subscription)) = line.split_once('\t') else {
            continue;
        };

        match tenants.iter_mut().find(|(t, _)| t == tenant) {
            Some((_, subscriptions)) => subscriptions.push(subscription.to_string()),
            None => tenants.push((tenant.to_string(), vec![subscription.to_string()])),
        }
    }

    // If there's a configured tenant, search that first - it's usually the right one, and
    // every tenant we skip is a token request saved.
    if let Some(preferred) = preferred
        && let Some(pos) = tenants.iter().position(|(t, _)| t == preferred)
    {
        tenants.swap(0, pos);
    }

    trace!("Tenants to search: {tenants:?}");
    tenants
}

/// Exchange an Azure AD access token for an ACR refresh token.
async fn exchange_token(registry: &str, tenant: &str, token: &Secret) -> StdResult<String> {
    // Need to connect to the repository's OAuth endpoint to exchange the token we just got.
    let url = Url::parse(&format!("https://{registry}/oauth2/exchange"))?;

    // Set up the parameters for the post to the OAuth endpoint as per
    // https://github.com/Azure/acr/blob/main/docs/AAD-OAuth.md#calling-post-oauth2exchange-to-get-an-acr-refresh-token
    let params = [
        ("grant_type", "access_token"),
        ("service", registry),
        ("tenant", tenant),
        ("access_token", token.secret()),
    ];
    trace!("Params: {params:?}");

    let form_body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();

    // Send the request to the endpoint in order to get the ACR refresh token.
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_body)
        .send()
        .await?
        .error_for_status()?
        .json::<AcrRefreshToken>()
        .await?;

    Ok(response.refresh_token)
}

#[cfg(test)]
mod tests {
    use super::subscriptions_by_tenant;

    #[test]
    fn preferred_tenant_is_searched_first() {
        // Requires a logged-in Azure CLI; skip otherwise.
        let discovered = subscriptions_by_tenant(None);
        if discovered.len() < 2 {
            return;
        }

        // Promoting the last tenant must move it to the front without
        // dropping any tenant or any of their subscriptions.
        let last = discovered.last().unwrap().0.clone();
        let ordered = subscriptions_by_tenant(Some(&last));

        assert_eq!(ordered[0].0, last);
        assert_eq!(ordered.len(), discovered.len());
        for (tenant, subscriptions) in &discovered {
            let found = ordered.iter().find(|(t, _)| t == tenant).unwrap();
            assert_eq!(&found.1, subscriptions);
        }
    }
}
