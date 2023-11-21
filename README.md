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
    "credsHelper": {
        "<registry>.azurecr.io": "acr-login"
    }
}
```
to handle requests for a specific registry.

## Required configuration

Before running Docker, you must:

- set the environment variable `AZURE_TENANT_ID` to the tenant ID that the ACR resides in
- ensure that you have Azure credential details set up; e.g. one of
    - logged into Azure CLI using `az login`
    - set `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` environment variables with appropriate values
    - any other method as per [DefaultAzureCredential](https://docs.rs/azure_identity/0.17.0/azure_identity/struct.DefaultAzureCredential.html)
- ensure that whichever identity you are using has `AcrPull` and `Reader` on the ACR (to pull) and `AcrPush` (to push)

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
