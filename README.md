# docker-credential-acr-login

A docker credential helper for Azure Container Registries (ACRs). This allows you to automatically log into Azure Container Registries without having to `az acr login` first.

## How do I use it?

Install it with

```shell
cargo install docker-credential-acr-login
```
and ensure `docker-credential-acr-login` is available on your PATH.

Next, in `$HOME/.docker/config.json`, either add:

```json
{
    "credsStore": "acr-login"
}
```
to handle requests for all Azure registries, or
```json
{
    "credHelpers": {
        "<registry>.azurecr.io": "acr-login"
    }
}
```
to handle requests for a specific registry.

## Required configuration

Before running Docker, you must:

- ensure that you have Azure credential details set up; e.g. one of
    - logged into Azure CLI using `az login`
    - set `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` environment variables with appropriate values
    - any other method as per [DefaultAzureCredential](https://docs.rs/azure_identity/0.17.0/azure_identity/struct.DefaultAzureCredential.html)
- ensure that whichever identity you are using has `AcrPull` and `Reader` on the ACR (to pull) and `AcrPush` (to push)

## Multiple tenants

An ACR only accepts a token issued by the tenant it lives in, so the helper
looks the registry up in [Azure Resource Graph][graph] to find which
subscription hosts it, then authenticates as that subscription's identity.
Graph queries are scoped to one tenant at a time, so each tenant you are signed
into is searched in turn until the registry is found.

Authenticating by subscription rather than by tenant is what makes this work
when you are signed in as several identities at once: a tenant alone resolves
to whichever account is currently active, which may have no access to the
tenant that owns the registry.

`AZURE_TENANT_ID` is optional and now only an optimisation: the tenant it names
is searched first.

The lookup needs `Reader` on the registry. Without it the registry simply isn't
visible in Resource Graph, so the helper falls back to attempting the token
exchange against each tenant in turn until one is accepted. `AcrPull` alone is
enough to pull, as the exchange endpoint doesn't check ARM permissions. The
lookup is therefore an optimisation that avoids failed exchanges, not a
requirement.

If no tenant works, the error lists every tenant tried and why each failed; the
usual fix is `az login --tenant <id>` for the tenant that owns the registry.

[graph]: https://learn.microsoft.com/en-us/azure/governance/resource-graph/overview

## Logging

Before running docker operations you can set the logging level by setting the environment variable `ACR_LOGIN_LOG_LEVEL` to one of `error`, `warn`, `info`, `debug`, or `trace`.

Example `trace` output:

```shell
$ docker pull dockercredentialacrlogin.azurecr.io/python:3.8-alpine
TRACE - Params: [("grant_type", "access_token"), ("service", "dockercredentialacrlogin.azurecr.io"), ("tenant", "<tenant>"), ("access_token", "eyJ...qiw")]
TRACE - Credentials: Object {"Secret": String("eyJ...beA"), "Username": String("000...000")}
3.8-alpine: Pulling from python
Digest: sha256:c494835919a916a1b1248eebe11815ada264e7b6b29f8784060c5f39b20b4747
Status: Downloaded newer image for dockercredentialacrlogin.azurecr.io/python:3.8-alpine
dockercredentialacrlogin.azurecr.io/python:3.8-alpine
```

## Minimum supported Rust version (MSRV)

This project's current MSRV is **1.88.0**.
