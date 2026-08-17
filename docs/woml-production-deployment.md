# Deploying WOML Production Runtime v1

Production Runtime v1 is a continuous, single-machine runtime. It uses local
persistent SQLite storage and permits exactly one active owner for a deployment.
It is suitable for a VPS, one container, or one Kubernetes pod backed by
`ReadWriteOnce` storage. It is not a multi-node or high-availability profile.

## Prepare and verify

Install Bun 1.3.14 or later and the WOML CLI, create an unprivileged runtime
user, and place `.woml`, module, and runtime configuration files in one owned
application directory. Keep the SQLite database, logs, and backups outside the
package installation directory.

Before starting or upgrading, run:

```bash
woml --version
woml check workflows/ --config woml.runtime.json
```

Production secrets should use owner-only mounted files or the reviewed
environment provider. Do not put values in `.woml`, runtime JSON, container
images, unit files, source control, or command-line arguments.

## systemd on a VPS

The reference unit is
[`examples/production/deployment/woml.service`](../examples/production/deployment/woml.service).
Install the application at `/srv/woml`, make `data`, `logs`, and `backups`
writable only by the `woml` user, install the unit, then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now woml
sudo systemctl status woml
journalctl -u woml -f
```

Run WOML in the foreground under systemd. Do not add `--background`; systemd is
the supervisor and must own the actual runtime process.

## Docker

Build the reference image from `examples/production`:

```bash
docker build -f deployment/Dockerfile -t my-woml:1.0.0 .
docker run --name woml --restart unless-stopped \
  -p 127.0.0.1:3000:3000 \
  -v woml-data:/app/data \
  -v /secure/woml-secrets:/run/secrets/woml:ro \
  -e WOML_SECRETS_PROVIDER=files \
  -e WOML_SECRETS_DIRECTORY=/run/secrets/woml \
  my-woml:1.0.0
```

Pin both the Bun image and WOML version. The image runs as a non-root user and
stores durable state on a separate volume. Copy verified backup directories to
another machine or object store; a container volume alone is not a backup.

## Reverse proxy and TLS

Expose only the public trigger listener. Keep Runtime Admin v1 on loopback and
use the local `woml inspect`, `list`, `get`, `cancel`, and `stop` commands rather
than proxying it. The reference Nginx configuration terminates TLS and proxies
to `127.0.0.1:3000`. Configure request/time limits at least as strict as WOML’s
frozen ingress limits and preserve webhook provider signature bytes.

## Single-pod Kubernetes

The reference manifest deliberately uses:

- `replicas: 1` and `Recreate`, never rolling overlap;
- one `ReadWriteOnce` persistent volume;
- exec probes against the loopback health listener;
- mounted Kubernetes Secret files;
- non-root, read-only-root-filesystem, resource-limit settings; and
- no public Runtime Admin service.

Apply it only after replacing the example image and secret names:

```bash
kubectl apply -f deployment/kubernetes.yaml
```

Kubernetes restarts the process but does not turn local SQLite into distributed
storage. Do not scale this deployment beyond one pod and do not use a shared
network filesystem unsupported by SQLite.

## Backup, restore, retention, and upgrade

Create and export verified backups regularly:

```bash
woml backup backups/$(date +%F)
woml prune --before 30d --dry-run
woml prune --before 30d
```

For an upgrade:

1. run `woml check` with the new binary;
2. create and copy a verified backup off-host;
3. stop the old owner gracefully;
4. install the pinned new version;
5. start it and wait for readiness/recovery;
6. inspect health, failed runs, and provider state; and
7. keep the old binary and backup until the acceptance window closes.

Restore is offline. Follow
[WOML Backup, Restore, and Store Upgrades](woml-backup-and-restore.md); never
copy the live SQLite/WAL files manually.

## Monitoring and alerting

Runtime Admin v1 metrics are authenticated and loopback-only. Run the collector
on the same host and obtain the current rotating capability from the owner-only
descriptor. Monitor readiness, failures, queued/waiting work, worker restarts,
store size, backups, and retention. A reference Prometheus rule group is in
`examples/production/deployment/prometheus-alerts.yaml`.

Use `woml inspect` for live human diagnosis. Logs should be collected from the
configured directory or supervisor stream and retained according to a separate
operational policy.

## Security checklist

- Run one unprivileged OS identity per deployment.
- Restrict workflow/module/config ownership and write access.
- Keep public ingress behind TLS and provider authentication.
- Keep admin on loopback and descriptor permissions owner-only.
- Use mounted secret files or a reviewed secret provider.
- Apply CPU, memory, process, file-descriptor, filesystem, and network limits.
- Treat authored JavaScript as trusted application code, not hostile tenant code.
- Back up off-host, test restore, and monitor free disk space.
- Review source and module changes before atomic activation.
- Never place secret or customer payload values in lifecycle messages or logs.

## Failure behavior

Low disk, a read-only path, corrupt SQLite, missing secrets, provider startup
failure, source changes, or a live competing owner all fail activation before
readiness. A slow graceful shutdown closes admission first and fails ambiguous
effects closed. Clock movement does not rewrite durable occurrence identities;
schedule/interval recovery follows the frozen missed-occurrence policy.
