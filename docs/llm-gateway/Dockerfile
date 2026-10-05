# Noetis LLM gateway image: LiteLLM plus our config and Entra auth hook.
# Pin LITELLM_IMAGE to a release tag or digest you have tested; don't run a moving tag in production.
ARG LITELLM_IMAGE=ghcr.io/berriai/litellm:main-stable
FROM ${LITELLM_IMAGE}
COPY config.yaml custom_auth.py /app/
CMD ["--config", "/app/config.yaml", "--port", "4000"]
