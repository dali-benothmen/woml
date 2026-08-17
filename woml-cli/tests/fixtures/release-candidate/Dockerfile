FROM oven/bun:1.3.14

ARG MAIN_ARCHIVE
ARG NATIVE_ARCHIVE
COPY ${MAIN_ARCHIVE} /tmp/woml.tgz
COPY ${NATIVE_ARCHIVE} /tmp/woml-native.tgz
RUN bun add --global /tmp/woml.tgz /tmp/woml-native.tgz \
  && rm /tmp/woml.tgz /tmp/woml-native.tgz

RUN addgroup --system woml && adduser --system --ingroup woml --home /app woml
WORKDIR /app
COPY --chown=woml:woml woml-cli/tests/fixtures/release-candidate/workflow.woml ./workflow.woml
COPY --chown=woml:woml woml-cli/tests/fixtures/release-candidate/runtime.json ./runtime.json
RUN mkdir -p /app/data /app/logs && chown -R woml:woml /app

USER woml
EXPOSE 3000
VOLUME ["/app/data"]
ENTRYPOINT ["woml"]
CMD ["run", "workflow.woml", "--config", "runtime.json"]
