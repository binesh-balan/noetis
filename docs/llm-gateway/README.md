# Noetis LLM Gateway (self-hosted LiteLLM)

Employees never get direct access to the model. Noetis talks only to an internal gateway that
checks each request's Entra ID token (employees only), applies a per-person rate limit and
monthly budget, logs spend per person, and calls Azure AI Foundry with a key that stays on
the server.

```
 Noetis (employee PC)                     Internal network                        Azure
┌──────────────────────┐  Bearer <Entra  ┌─────────────────────────────┐  api-key  ┌──────────────────┐
│ signs in with Entra  │  access token>  │ LiteLLM gateway             │ (server-  │ AI Foundry       │
│ (work account, PKCE) ├────────────────►│ 1. custom_auth.py: token is │  side     │ DeepSeek         │
└──────────────────────┘     HTTPS       │    ours, for the gateway,   │  secret)  │ key auth + no    │
                                         │    has role Noetis.User     ├──────────►│ public access    │
                                         │ 2. per-user TPM/RPM limits  │           │ (private         │
                                         │ 3. per-user monthly budget  │           │  endpoint)       │
                                         │ 4. spend logged per user    │           └──────────────────┘
                                         └──────────────┬──────────────┘
                                                        │ Postgres: users, budgets, spend
```

**Nothing changes in the Noetis app.** It already sends its Entra token as a Bearer header to
the configured endpoint; IT only points `policy.json` at the gateway with the gateway's scope.

## Why a custom auth hook

LiteLLM's built-in JWT/OIDC sign-in needs a LiteLLM **Enterprise licence**. Its custom auth
hook is **open source**, so `custom_auth.py` (about 40 lines) validates the Entra token itself
with PyJWT: signature against Microsoft's published keys, issuer and tenant, audience (the
gateway app only), expiry, and the `Noetis.User` app role. `test_custom_auth.py` checks that it
rejects wrong-tenant, wrong-audience, role-less, expired, forged and unsigned tokens.

## Files

| File | Purpose |
|---|---|
| `custom_auth.py` | Entra ID token check; sets the user id (Entra object id) and rate limits |
| `config.yaml` | LiteLLM config: the DeepSeek model, prices, auth hook, budget enforcement |
| `Dockerfile` | LiteLLM image plus `config.yaml` and `custom_auth.py` |
| `docker-compose.yml`, `.env.example` | Pilot on one internal server (gateway + Postgres) |
| `sync-users.ps1` | Gives each employee a LiteLLM user record with a monthly budget (run hourly) |
| `test_custom_auth.py` | Validation tests (`pip install "pyjwt[crypto]" pytest`, then `pytest`) |

---

## 1. Entra ID: employees only

1. **App registrations → New registration** "Noetis LLM Gateway" (single tenant). No redirect URI.
2. **Expose an API**: set the Application ID URI to `api://<gateway app id>`.
3. **App roles → Create**: display name *Noetis User*, value `Noetis.User`, allowed member
   types *Users/Groups*.
4. **Manifest**: set `"requestedAccessTokenVersion": 2` (tokens then carry the app id as `aud`).
5. **Enterprise applications → Noetis LLM Gateway → Properties**: **Assignment required = Yes**.
   **Users and groups**: assign your employee group to the *Noetis User* role. Nobody else can
   get a token for the gateway.
6. **The existing Noetis app registration** (the one in `policy.json` `entra.clientId`):
   **API permissions → Add → My APIs → Noetis LLM Gateway**, then grant admin consent.
7. Recommended: a **Conditional Access** policy targeting *Noetis LLM Gateway* that requires
   MFA and a compliant (Intune-managed) device, so a valid account still can't use the
   gateway from a personal machine.

## 2. Azure AI Foundry: no direct access

- Remove any user or group role assignments on the Foundry resource; only the gateway calls it.
- **Networking**: disable public network access and add a private endpoint in the gateway's
  network (or allow only the gateway's outbound IP).
- Keep the Foundry key in Key Vault (or `.env` for the pilot) and rotate it there.

## 3. Deploy the gateway

### Pilot: one internal server (Docker)

```bash
cd docs/llm-gateway
cp .env.example .env      # fill in the values
docker compose up -d --build
```

The gateway listens on `127.0.0.1:4000`. Put it behind your internal HTTPS reverse proxy with a
certificate your PCs trust, e.g. `https://llm-gateway.contoso.internal`.

### Production: Azure Container Apps

- Build `Dockerfile` into your Azure Container Registry, with `LITELLM_IMAGE` **pinned to a
  tested release tag or digest**.
- Container Apps environment in a VNet with **internal ingress only** (reachable from the
  corporate network/VPN), target port 4000, at least 2 replicas.
- **Azure Database for PostgreSQL Flexible Server** (private access) for `DATABASE_URL`.
- Secrets (`AZURE_AI_API_KEY`, `LITELLM_MASTER_KEY`, `DATABASE_URL`) as Key Vault references;
  the rest of `.env.example` as plain environment variables.
- Private DNS name and an internal certificate, e.g. `https://llm-gateway.contoso.internal`.

Keep the admin endpoints (`/user/*`, `/key/*`, `/ui`) internal; only the master key can use them.

## 4. Budgets and rate limits

- **Rate limits** (every request): `USER_TPM_LIMIT` tokens and `USER_RPM_LIMIT` requests per
  minute per person, from `.env`.
- **Monthly budget** (USD per person): LiteLLM prices each request with
  `input_cost_per_token` / `output_cost_per_token` in `config.yaml` (set these from your Azure
  price sheet) and stops a person's requests once their budget for the period is used.
  `sync-users.ps1` creates or updates each employee's record:

  ```powershell
  $env:LITELLM_MASTER_KEY = '<master key>'
  .\sync-users.ps1 -GatewayUrl https://llm-gateway.contoso.internal -GroupId <employee group id> -MonthlyBudgetUsd 10
  ```

  Schedule it hourly (Task Scheduler or an Azure Automation runbook with a managed identity
  that can read group members). New hires are rate-limited from their first request and get
  their budget at the next sync. Leavers lose access immediately when their account or role
  assignment is removed (no valid token), whatever the sync says.

- To give one person a different budget: `POST /user/update` with their Entra object id as
  `user_id` and a new `max_budget` (master key), and leave them out of the sync's group or
  run the sync for groups with different `-MonthlyBudgetUsd`.

## 5. Point Noetis at the gateway

```powershell
.\deploy-policy.ps1 -Endpoint "https://llm-gateway.contoso.internal/v1" -Model "noetis-deepseek" `
  -TenantId "<directory id>" -ClientId "<noetis application id>" `
  -Scope "api://<gateway application id>/.default" -ModelsSource "\\fileserver\noetis-models" -Language en
```

This writes `summary.endpoint`, `summary.model` and `entra.scope` into `policy.json`. Summaries
and live answers both go through the gateway; users sign in once with their work account.

## 6. Verify

```bash
GW=https://llm-gateway.contoso.internal
# No token / someone else's token: 401
curl -s -o /dev/null -w "%{http_code}\n" $GW/v1/chat/completions -H "Content-Type: application/json" -d '{"model":"noetis-deepseek","messages":[{"role":"user","content":"hi"}]}'
# An employee token (az login as an assigned employee first): 200
TOKEN=$(az account get-access-token --scope "api://<gateway app id>/.default" --query accessToken -o tsv)
curl -s $GW/v1/chat/completions -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"model":"noetis-deepseek","messages":[{"role":"user","content":"Say hello"}]}'
# Spend so far for that person (master key)
curl -s "$GW/user/info?user_id=<their object id>" -H "Authorization: Bearer $LITELLM_MASTER_KEY"
```

For the `az` token step, add the Azure CLI's client id (`04b07795-8ddb-461a-bbee-02f9e1bf7b46`) as an
authorized client application under the gateway's **Expose an API**, or test from Noetis itself.

Then in Noetis: sign in under Settings → Organization account, generate a summary, and check
that spend appears for that user. Test a budget by setting a tiny `max_budget` for yourself.

## 7. Operating it

- **Privacy**: LiteLLM records spend per request. Keep prompt and response logging **off**
  (the default); meeting transcripts are sensitive. If you add a logging callback, make sure it
  doesn't capture message content.
- **Upgrades**: bump the pinned LiteLLM tag in a test environment first, run
  `test_custom_auth.py` and the checks in section 6, then roll out.
- **Rotation**: the Foundry key and master key live only on the gateway; rotate them there with
  no change on any PC.
- **Monitoring**: per-user spend via `/user/info`, `/spend/logs`, or the LiteLLM admin UI on
  the internal network.

## What this does and doesn't stop

It stops: anyone outside your tenant or not assigned the role; direct use of Foundry; use from
unmanaged devices (with Conditional Access); unlimited spend by one person.

It doesn't stop an employee from copying their own token out of Noetis and calling the gateway
with their own prompts. No gateway can tell the app from someone holding a valid token, but
the damage is contained: they reach only this one model, within their budget and rate limit,
and every call is logged against their name.

## Decisions to make

- Monthly budget per person, and per-minute limits.
- Whether everyone gets the same budget, or groups (e.g. heavy users) get more.
- Pilot server vs Azure Container Apps for production.
- Whether to require compliant devices via Conditional Access.
