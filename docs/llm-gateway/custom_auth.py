"""Noetis LLM gateway: Entra ID sign-in for LiteLLM's open-source custom auth hook.

Accepts only access tokens that our tenant issued for the gateway app registration and
that carry the gateway's app role. With "Assignment required" on the gateway's enterprise
app, only assigned employees can get such a token. LiteLLM tracks spend and enforces
budgets per user id (the Entra object id); rate limits are set here.

LiteLLM's built-in JWT auth needs an Enterprise licence; this hook does the same check
with PyJWT, which ships with the LiteLLM proxy image.
"""

import os

import jwt
from fastapi import Request
from litellm.proxy._types import UserAPIKeyAuth

TENANT_ID = os.environ["ENTRA_TENANT_ID"]
# Gateway app's Application ID (v2 tokens) and/or Application ID URI (v1 tokens), comma-separated.
AUDIENCES = [a.strip() for a in os.environ["ENTRA_API_AUDIENCE"].split(",") if a.strip()]
REQUIRED_ROLE = os.environ.get("ENTRA_REQUIRED_ROLE", "Noetis.User")
ISSUERS = {
    f"https://login.microsoftonline.com/{TENANT_ID}/v2.0",
    f"https://sts.windows.net/{TENANT_ID}/",
}
USER_TPM_LIMIT = int(os.environ.get("USER_TPM_LIMIT", "20000"))
USER_RPM_LIMIT = int(os.environ.get("USER_RPM_LIMIT", "30"))
ALLOWED_MODELS = [m.strip() for m in os.environ.get("ALLOWED_MODELS", "noetis-deepseek").split(",") if m.strip()]

_jwks = jwt.PyJWKClient(
    f"https://login.microsoftonline.com/{TENANT_ID}/discovery/v2.0/keys",
    cache_keys=True,
)


def verify(token: str, signing_key=None) -> dict:
    """Claims of a valid employee access token; raises on anything else."""
    key = signing_key or _jwks.get_signing_key_from_jwt(token).key
    claims = jwt.decode(
        token,
        key,
        algorithms=["RS256"],
        audience=AUDIENCES,
        options={"require": ["exp", "iat", "iss", "aud", "tid", "oid"]},
        leeway=60,
    )
    if claims["iss"] not in ISSUERS or claims["tid"] != TENANT_ID:
        raise PermissionError("token is not from this organization")
    if REQUIRED_ROLE not in claims.get("roles", []):
        raise PermissionError(f"missing app role {REQUIRED_ROLE}")
    return claims


async def user_api_key_auth(request: Request, api_key: str) -> UserAPIKeyAuth:
    try:
        claims = verify(api_key)
    except Exception as e:  # noqa: BLE001 - any failure is a 401
        # With custom_auth_settings.mode "auto", LiteLLM then tries its own keys (the admin
        # master key); anything else ends in 401.
        raise Exception(f"Unauthorized: {e}")
    return UserAPIKeyAuth(
        api_key=api_key,
        user_id=claims["oid"],
        user_tpm_limit=USER_TPM_LIMIT,
        user_rpm_limit=USER_RPM_LIMIT,
        models=ALLOWED_MODELS,
        metadata={"upn": claims.get("preferred_username") or claims.get("upn", "")},
    )
