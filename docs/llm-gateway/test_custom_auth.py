"""Checks the gateway's token validation without LiteLLM or network access.

    pip install "pyjwt[crypto]" pytest
    pytest docs/llm-gateway/test_custom_auth.py
"""

import os
import sys
import time
import types

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa

TENANT = "11111111-1111-1111-1111-111111111111"
AUD = "22222222-2222-2222-2222-222222222222"
os.environ.update(ENTRA_TENANT_ID=TENANT, ENTRA_API_AUDIENCE=f"{AUD},api://noetis-gateway")

# Stand-ins for the LiteLLM/FastAPI imports, so only the validation logic is under test.
sys.modules.setdefault("fastapi", types.SimpleNamespace(Request=object))
_types = types.ModuleType("litellm.proxy._types")
_types.UserAPIKeyAuth = lambda **kw: kw
for name in ("litellm", "litellm.proxy"):
    sys.modules.setdefault(name, types.ModuleType(name))
sys.modules["litellm.proxy._types"] = _types
sys.path.insert(0, os.path.dirname(__file__))
import custom_auth  # noqa: E402

KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
OTHER_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)


def token(key=KEY, **overrides):
    now = int(time.time())
    claims = {
        "iss": f"https://login.microsoftonline.com/{TENANT}/v2.0",
        "aud": AUD,
        "tid": TENANT,
        "oid": "user-oid-1",
        "preferred_username": "alex@contoso.com",
        "roles": ["Noetis.User"],
        "iat": now,
        "exp": now + 3600,
    }
    claims.update(overrides)
    claims = {k: v for k, v in claims.items() if v is not None}
    return jwt.encode(claims, key, algorithm="RS256")


def verify(t):
    return custom_auth.verify(t, signing_key=KEY.public_key())


def test_accepts_an_employee_token():
    assert verify(token())["oid"] == "user-oid-1"
    assert verify(token(aud="api://noetis-gateway"))["oid"] == "user-oid-1"
    assert verify(token(iss=f"https://sts.windows.net/{TENANT}/"))["oid"] == "user-oid-1"


@pytest.mark.parametrize(
    "bad",
    [
        {"aud": "https://cognitiveservices.azure.com"},  # token for another resource
        {"tid": "99999999-9999-9999-9999-999999999999"},  # another tenant
        {"iss": "https://login.microsoftonline.com/common/v2.0"},
        {"roles": ["Something.Else"]},  # not assigned to the gateway
        {"roles": None},
        {"oid": None},
        {"exp": int(time.time()) - 3600},  # expired
    ],
)
def test_rejects_anything_else(bad):
    with pytest.raises(Exception):
        verify(token(**bad))


def test_rejects_a_token_signed_by_someone_else():
    with pytest.raises(Exception):
        verify(token(key=OTHER_KEY))


def test_rejects_unsigned_tokens():
    unsigned = jwt.encode({"aud": AUD, "tid": TENANT, "oid": "x", "roles": ["Noetis.User"]}, None, algorithm="none")
    with pytest.raises(Exception):
        verify(unsigned)
