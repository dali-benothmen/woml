FROM oven/bun:1.3.14

ARG WOML_VERSION=1.0.8
RUN bun add --global "woml-cli@${WOML_VERSION}"

RUN addgroup --system woml && adduser --system --ingroup woml --home /app woml
WORKDIR /app
COPY --chown=woml:woml complete/workflows ./workflows
COPY --chown=woml:woml complete/modules ./modules
COPY --chown=woml:woml deployment/woml.docker.runtime.json ./woml.runtime.json
RUN mkdir -p /app/data /app/logs /app/backups && chown -R woml:woml /app

USER woml
EXPOSE 3000
VOLUME ["/app/data", "/app/backups"]
ENTRYPOINT ["woml"]
CMD ["run", "workflows/", "--config", "woml.runtime.json"]
