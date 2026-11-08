# Cloudflare Anycast Ingress Deployment Guide

This guide details configuring **Cloudflare Tunnel (`cloudflared`)** for AndroGrid cellular worker ingress. This architecture enables remote Android worker nodes on metered cellular WAN networks to connect to the host control plane without public static IPs or router port-forwarding rules.

Traceability: `[ANTI-002]`, `[REQ-OP-003]`, `[INV-001]`, `[INV-002]`, `[NFR-SEC-001]`.

---

## Architectural Context and Anti-Requirement Compliance

Traditional self-hosted edge clusters require operators to configure dynamic DNS, public static IP addresses, or router NAT port-forwarding. In AndroGrid:

- **[ANTI-002] Anti-Requirement:** The deployment architecture shall not require router port-forwarding rules, public static IP addresses, or third-party client VPN configuration on the host infrastructure.
- **[INV-001] Outbound Ingress:** The `cloudflared` daemon creates an outbound-only QUIC/HTTP2 tunnel connection to Cloudflare Anycast edge servers.
- **Multi-Hop Transport Boundaries:** WAN cellular traffic traverses three distinct transport segments with decoupled TLS boundaries.

```mermaid
flowchart LR
    subgraph MobileDevice["Remote Cellular Worker"]
        Worker["Android Client<br/>(Metered WAN)"]
    end

    subgraph AnycastEdge["Cloudflare Anycast Edge"]
        CFEdge["Cloudflare Edge Network<br/>grpc.cluster.domain.com<br/>(Edge TLS Termination)"]
    end

    subgraph HostPremise["Host Local Environment (Private LAN)"]
        Tunnel["cloudflared Tunnel Daemon<br/>(Outbound Tunnel Session)"]
        Orchestrator["Rust Host Control Plane<br/>https://orchestrator:50051"]
    end

    Worker -->|"Leg 1: WAN Ingress<br/>TLS 1.3 to Edge<br/>(API Shield mTLS Optional)"| CFEdge
    CFEdge -->|"Leg 2: Multiplexed Stream<br/>(Encrypted QUIC / HTTP2 Tunnel)"| Tunnel
    Tunnel -->|"Leg 3: Origin HTTP/2 Transport<br/>(TLS Verified with ca.crt)"| Orchestrator
```

### Ingress Transport Legs and Authentication Boundaries

1. **Leg 1 (Mobile WAN Ingress):** The Android worker connects outbound to Cloudflare Anycast edge servers over TLS 1.3. Standard Cloudflare Anycast terminates the external TLS session at the edge.
2. **Leg 2 (Anycast Ingress Tunnel):** The `cloudflared` daemon maintains an outbound, persistent multiplexed QUIC/HTTP2 session to Cloudflare, pulling incoming traffic without exposing open inbound host ports (`[INV-001]`, `[ANTI-002]`).
3. **Leg 3 (Host Local Origin Transport):** The `cloudflared` daemon proxies requests locally to the Rust orchestrator (`https://orchestrator:50051`). Origin verification is enforced by `noTLSVerify: false` and `caPool: /etc/cloudflared/ca.crt`, ensuring `cloudflared` only forwards traffic to an orchestrator presenting a valid internal certificate signed by the cluster Root CA.

> [!IMPORTANT]
> **Edge vs Origin mTLS Boundary:** Standard Cloudflare Tunnels terminate the client's TLS session at Cloudflare's Anycast edge; client X.509 certificates are not transparently forwarded through the tunnel stream to the origin gRPC server. While direct Wi-Fi connections terminate mTLS 1.3 directly at the Rust orchestrator (`[INV-002]`), WAN ingress via Cloudflare requires provisioning **Cloudflare API Shield mTLS** at the edge or evaluating application-layer cryptographic tokens to enforce mobile node identity before dispatching tasks. Cloudflare diagrams must reflect this three-hop boundary rather than asserting uninterrupted end-to-end client-certificate forwarding.

---

## Step-by-Step Setup

### Step 1: Install and Authenticate `cloudflared`

Install the `cloudflared` binary on the host machine or run the container image via Podman:

```bash
# Authenticate cloudflared with your Cloudflare account
cloudflared tunnel login
```

This generates an authorization certificate in `~/.cloudflared/cert.pem`.

---

### Step 2: Create an Ingress Tunnel

Create a named tunnel for the cluster:

```bash
cloudflared tunnel create androgrid-control-plane
```

Take note of the returned Tunnel UUID. A credentials file is created at:
`~/.cloudflared/<TUNNEL_UUID>.json`.

Copy this credentials file into the `deploy/` directory:
```bash
cp ~/.cloudflared/<TUNNEL_UUID>.json ./deploy/credentials.json
```

---

### Step 3: Route DNS to the Ingress Tunnel

Bind your chosen public gRPC hostname to the tunnel:

```bash
cloudflared tunnel route dns androgrid-control-plane grpc.cluster.domain.com
```

---

### Step 4: Configure `cloudflared.yml`

Copy the configuration template from `deploy/ingress/cloudflared-template.yml` to `deploy/ingress/cloudflared.yml`:

```yaml
tunnel: <REPLACE_WITH_TUNNEL_UUID>
credentials-file: /etc/cloudflared/credentials.json

ingress:
  # Route gRPC WAN traffic to local Rust orchestrator control plane
  - hostname: grpc.cluster.domain.com
    service: https://orchestrator:50051
    originRequest:
      http2Origin: true
      noTLSVerify: false
      caPool: /etc/cloudflared/ca.crt

  # Default catch-all service returns HTTP 404 for unmatched hostnames
  - service: http_status:404
```

Key configuration parameters:
- `http2Origin: true`: Mandatory for gRPC multiplexing.
- `noTLSVerify: false`: Enforces certificate validation against the local Root CA (`ca.crt`).
- `caPool: /etc/cloudflared/ca.crt`: Internal Root CA certificate ensuring trusted origin communication.

---

### Step 5: Launch Ingress via Podman Compose

In `deploy/podman-compose.yml`, the `cloudflared` service is configured as a companion container:

```bash
cd deploy
podman compose up -d cloudflared
```

Inspect tunnel logs to confirm established edge connections:

```bash
podman compose logs -f cloudflared
```

Expected log output:
```text
INF Connection registered with Cloudflare edge location: ORD
INF Connection registered with Cloudflare edge location: IAD
```

---

## Verification

Test the ingress tunnel from a remote network using `androgrid-cli`:

```bash
androgrid-cli \
  --endpoint https://grpc.cluster.domain.com \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  nodes
```

The request traverses the Cloudflare Anycast edge, forwards through the tunnel stream to the origin orchestrator over verified origin TLS, and returns the cluster node inventory.
