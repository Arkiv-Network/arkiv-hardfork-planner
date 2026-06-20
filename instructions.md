# Atlas Network Integration

Run `arkiv-hardfork-planner` as a small HTTP service next to the Atlas network services. It replaces the old static `protocol-schedule` sidecar; do not run both services with the same `protocol-schedule` DNS role on the Atlas Docker network.

Atlas clients should read the published schedule from:

```text
http://protocol-schedule:28882/arkiv-protocol-schedule.json
```

Use the Atlas chain ID in both the schedule file and the service config. The default schedule in this repository uses `chainId: 42069`.

## Required Runtime Configuration

```env
LISTEN_HOST=0.0.0.0
LISTEN_PORT=28882
HTML_TITLE="Atlas Hardfork Planner"
SCHEDULE_PATH=/data/arkiv-protocol-schedule.json
CHAIN_ID=42069
RPC_URL=http://reth:8545
RPC_STARTUP_MODE=deferred
RPC_POLL_SECONDS=10
RPC_TIMEOUT_MS=5000
ADMIN_BEARER_KEY=<optional strong random admin token>
```

`RPC_URL` must point at the execution node used by the Atlas network. `RPC_STARTUP_MODE` controls how strongly the service gates startup:

- `strict` is the default. On startup the planner calls `eth_chainId`; if the RPC chain does not match the schedule chain ID, or the check cannot complete, the service exits instead of publishing an unverified schedule.
- `deferred` is the Atlas compose mode. The planner starts serving the validated schedule immediately, then keeps checking RPC until it can verify the chain ID. This avoids a startup cycle where `reth` waits for `protocol-schedule` while the planner waits for `reth`.

Set reth's `.env` schedule URL to the planner-native port:

```env
ARKIV_PROTOCOL_SCHEDULE_URL=http://protocol-schedule:28882/arkiv-protocol-schedule.json
```

## Docker Compose Example

Use `docker-compose.atlas.yml` as the Atlas drop-in shape:

```yaml
services:
  protocol-schedule:
    image: ghcr.io/arkiv-network/arkiv-hardfork-planner:main
    ports:
      - "${PROTOCOL_SCHEDULE_PORT:-28882}:28882"
    environment:
      LISTEN_HOST: "0.0.0.0"
      LISTEN_PORT: "28882"
      HTML_TITLE: "Atlas Hardfork Planner"
      SCHEDULE_PATH: /data/arkiv-protocol-schedule.json
      CHAIN_ID: "42069"
      RPC_URL: http://reth:8545
      RPC_STARTUP_MODE: deferred
      RPC_POLL_SECONDS: "10"
      RPC_TIMEOUT_MS: "5000"
      ADMIN_BEARER_KEY: ${ADMIN_BEARER_KEY:-}
    volumes:
      - ./data:/data
    restart: unless-stopped
    networks:
      - atlas

networks:
  atlas:
    name: atlas
    external: true
```

Place the active schedule at `./data/arkiv-protocol-schedule.json`. Keep this file backed up; admin changes are persisted there before they are published in memory.

## Operating Notes

- Leave `ADMIN_BEARER_KEY` unset to run in read-only mode.
- Set `ADMIN_BEARER_KEY` only for trusted operators, and access the UI over a trusted network or TLS-terminated proxy.
- Configure Atlas nodes or tooling to consume `/arkiv-protocol-schedule.json`.
- Use `/healthz` for a lightweight liveness check and `/status` for current version, chain ID, current block, retained release history, and RPC verification fields (`rpcVerified`, `rpcChainId`, `rpcError`) when RPC polling is configured.
