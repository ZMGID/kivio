# FreeRoute

FreeRoute is Kivio's free-tier model aggregator: it registers as a normal provider (`kivio://freeroute`) and fails over across official free APIs before the first token is committed to the reply.

Routing, per-error cooldowns (auth 10 min / rate-limit 60 s / other exponential backoff), and first-token timeout failover follow ideas from the MIT project [dsh-freeroute](https://github.com/dushaobindoudou/dsh-freeroute). This is an independent port of those ideas into Kivio's provider, credential, and chat abstractions — not a wholesale copy of that codebase.

## Vendors

Only official free tiers:

- **OpenCode Zen** (`https://opencode.ai/zen/v1`) — anonymous `-free` / `big-pickle` models; no key required.
- **OpenRouter** — models whose id ends in `:free` (plus known $0 ids such as `stealth/union-alpha`). One API key.
- **SenseNova** — `sensenova-6.8-flash-lite` only (verified free on the official OpenAI-compatible endpoint). One API key.
- **Groq** — Groq's official free API. One API key.

B.AI is not included: its complimentary quota was observed exhausted in 2026-09.

## Keys

Exactly one API key per vendor. Keys are stored in `settings.json` on `freeRoute.vendors[].apiKey`, the same persistence path as other providers' `apiKeys`. They are never written to a sidecar plaintext config file. A multi-line paste is reduced to the first non-empty line.

## Failover

`auto` tries each ready vendor's default model in settings order. A pinned `vendor/model` is tried first, then the auto chain. Cooldowns:

- HTTP 401 / 402 / 403 → 10 minutes
- HTTP 429 → 60 seconds
- other errors (including first-token timeout, default 90 s) → exponential backoff from 30 s, capped at 10 minutes

A vendor that has already emitted tokens is not abandoned mid-stream.

Assistant messages record `served_vendor` and the upstream `model` so the chat UI can label which vendor answered.
