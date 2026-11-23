# Model provider account authorization

## Connecting a provider

1. In **Settings → Model providers**, add **ChatGPT**, **Codex OAuth**, **Kimi OAuth** or **Antigravity OAuth**. You can also choose an authorization method on an existing provider.
2. Sign in through the system browser. For ChatGPT, choose an account and workspace and grant Kivio permission to use your plan. For Codex OAuth or Kimi, enter the displayed device code. For Antigravity, complete Google sign-in.
3. Choose an available model and enable it. The ChatGPT catalog belongs to the current account. If a previously selected model is unavailable, select another model.
4. Test the selected model before using it for chat, translation or vision. For ChatGPT, **Manage usage** opens the [ChatGPT usage settings](https://chatgpt.com/settings/usage).

Account eligibility and quota are controlled by the upstream service; an OAuth login does not itself guarantee model access. These providers support text generation, tools and vision input through Kivio's existing model layer. The separate image-generation API is not enabled for these account sessions.

## Implementation

- `src-tauri/src/provider_oauth.rs`: bounded login sessions, server-directed polling intervals, expiry/cancellation, credential storage, refresh and model discovery.
- `ProviderRequestConfig.oauth`: `{ provider, credentialId?, requiresReauthorization? }`. This contains metadata only. API keys retain their existing storage and behavior; a selected OAuth mode never falls back silently to an old API key.
- Secrets use local `provider-oauth/<credentialId>.json` files, per the same plaintext JSON policy as API/MCP credentials. Writes use a private temporary file, fsync and atomic rename; Unix directories/files are restricted to 0700/0600. Existing versioned keyring credentials migrate on first use under a serialized owner: the legacy entry is removed only after the complete JSON replacement is durable. A failed write leaves the legacy entry intact; malformed existing JSON never revives an older token. Configuration ZIP backups include available OAuth credentials. Full chat backups contain conversations and attachments. See [backup and restore](backup.md). Restore imports under fresh references without replacing an existing account file; providers sharing one old credential reference still share one restored refresh authority. Configuration ZIPs contain plaintext secrets and should only be transferred between trusted devices. Backups lacking OAuth secrets require sign-in again.
- Concurrent refresh and disconnect operations are serialized. Refresh tokens remain in Kivio's credential store; the Hermes, Codex and Kimi credential files are neither imported nor modified.
- Credential resolution validates the exact provider endpoint and protocol before retrieving tokens. OAuth HTTP clients do not follow redirects. Errors do not include token endpoint response bodies or credentials.
- All normal model calls use the common model dispatcher. It resolves an ephemeral provider clone immediately before inference; refreshed access tokens are never written into settings or returned to the webview.
- ChatGPT uses OpenAI's dynamic public-client registration. Each installation keeps a stable host ID. A saved registration binds the issued client ID to the verified OpenID Connect subject. Kivio verifies the signed ID token and checks the plan permission before inference. [Registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in) and [accounts and sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions).
- ChatGPT model discovery and generation use the public `https://api.openai.com/v1` endpoint. Kivio sends the conversation context on each request, requests a stream with `store: false`, and waits for `response.completed`. Eligible models accept local function tools. [Models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference) and [preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations).
- Codex OAuth uses the [Codex device-code flow](https://github.com/openai/codex/blob/main/codex-rs/login/src/device_code_auth.rs) and the Codex backend for model discovery and streamed Responses. Kivio stores this authorization separately from Sign in with ChatGPT and the local Codex CLI session.
- Kimi uses its device authorization endpoint and the OpenAI-compatible coding endpoint, with Kimi client/device headers. Model lists are fetched from the authenticated provider rather than hard-coded.
- Antigravity uses Authorization Code + PKCE, an independent random state, and a temporary listener bound to `127.0.0.1:51121`. The browser redirect is `http://localhost:51121/oauth-callback`. Cancellation, a five-minute timeout, and completion release the listener; an occupied port produces a retryable error. Callback errors never echo codes or query strings.
- Antigravity project metadata is discovered with `loadCodeAssist` and kept with the credentials in the local JSON store. Missing projects require account setup in Antigravity followed by a new login. No project IDs are borrowed or synthesized. Refresh preserves the project and handles refresh-token rotation.
- Antigravity lists runtime IDs from `fetchAvailableModels`, preserving account-specific reasoning variants. Requests use its native Cloud Code Assist envelope; the Gemini adapter unwraps SSE responses and preserves tool IDs/signatures for Claude and GPT-OSS. Nonstreaming consumers collect the same stream, and incomplete streams report an error.
- Antigravity uses the daily Cloud Code Assist endpoint. Automatic endpoint/model substitution, remote callback pasting and built-in Google Search are not implemented. Normal agent search tools remain available. This is an unofficial integration; real account access and upstream compatibility still need live verification.
- Model discovery and connection tests accept unsaved authentication metadata, matching the rest of the settings workflow.

## Attribution

Kivio implements OpenAI's public Sign in with ChatGPT protocol in Rust and uses its existing local JSON credential store. The implementation follows the account lifecycle in [OpenAI DevKit 0a36fef](https://github.com/openai/sign-in-with-chatgpt-devkit/tree/0a36fefeb913055c8c7a1a29b63d82b96b2e841a/packages/local).

Protocol logic in `provider_oauth.rs` is adapted from Hermes Agent (Copyright © 2025 Nous Research, MIT) and Kimi CLI (Moonshot AI, Apache-2.0), rewritten for Tauri/Rust with Kivio-owned storage and UI. License texts are retained in [licenses/hermes-agent-MIT.txt](licenses/hermes-agent-MIT.txt) and [licenses/kimi-cli-Apache-2.0.txt](licenses/kimi-cli-Apache-2.0.txt).

`provider_oauth/antigravity.rs` and the Antigravity Gemini bridge adapt protocol details from Rahul Arya's pi-antigravity at commit `70e8f6e3603c4926e29d31c97be5f9719003f84f`. Its MIT license is retained in [licenses/pi-antigravity-MIT.txt](licenses/pi-antigravity-MIT.txt).

## Authorized account identity

Kivio displays the verified ChatGPT registration identity returned by OpenAI. Antigravity reads email, name and subject from Google's OpenID userinfo endpoint. Kimi displays the user ID and any available email or name. Only display metadata reaches the UI. Fetch failures can be retried independently of usage, and stale results are discarded when the account changes.

Kivio saves the issued ChatGPT client ID before exchanging the authorization code. If sign-in is interrupted, select the unfinished registration to continue with the same client ID. Returning sign-ins use the saved email as a browser hint. Kivio checks the verified subject and the saved registration revision before committing the result, so a later disconnect takes precedence.

When a refresh returns a new ID token, Kivio saves the replacement credentials and verifies the identity before using them. A temporary signing-key outage leaves the refresh pending. A later model request resumes verification, including after an app restart.

## Verification

Frontend tests exercise successful authorization, delayed polling, expiry errors, cancellation before the start response, and credential cleanup after unmount. Rust tests cover endpoint binding, ChatGPT request fields, refresh-token rotation parsing and both model-list shapes. JSON-store tests use a real temporary directory to verify durable migration, private Unix permissions, write-failure preservation, rotation, rejection of malformed JSON, and eight concurrent readers sharing a single legacy authority. The legacy keyring I/O is replaced with synthetic entries in those tests; no real user token or keychain entry is accessed.

Run frontend tests with `npx vitest run src/settings/ProviderOAuthPanel.test.tsx src/settings/tabs/ProvidersTab.test.tsx src/settings/providerRequest.test.ts src/onboarding/validation.test.ts`. On Windows run Rust tests with `./scripts/win-cargo-test.ps1 --lib provider_oauth`. On macOS use `cargo test --manifest-path src-tauri/Cargo.toml --lib provider_oauth`. The JSON migration cases are `json_credentials_migrate_durably_and_preserve_legacy_on_failure` and `concurrent_json_migration_reads_one_legacy_authority`. Real legacy keyring migration and Windows filesystem behavior still require platform acceptance with synthetic credentials.

Real account authorization and live model access require the account holder to complete the browser flow; mocked tests do not establish subscription access or service availability.

On 2026-11-23, a macOS Kivio Test installation completed browser authorization and listed 10 models for the consenting account. The provider connection test completed with `gpt-6.2-sol`. A built-in agent turn then invoked `read` once on a temporary file and returned its randomly generated contents exactly.

At the same account's natural renewal window, Kivio received a replacement refresh token and ID token, verified the original subject, and persisted the extended access lifetime. After restarting, Kivio listed all 10 visible models using the renewed session, with no second rotation.

Antigravity verification on 2026-09-05: 57 focused frontend tests and 44 Rust tests (10 OAuth, 25 Gemini, 9 request-header tests) passed. The callback tests use real loopback sockets with synthetic state and denial/cancellation; no Google credentials are exchanged. Adapter fixtures cover the request envelope, endpoint binding, bearer authentication, response unwrapping, tool IDs and thought signatures. TypeScript, changed-file ESLint, production UI build and protocol export check passed. Real Google authorization and live Antigravity inference have not been performed.

Historical keyring implementation verification on 2026-09-05: 56 focused frontend tests passed; 5 OAuth unit tests, 9 request-header tests and 37 Responses-adapter tests passed. The native Windows credential-store test was also explicitly executed and passed, including cleanup. TypeScript, changed-file ESLint, and the production frontend build passed. A browser preview exercised the device-code panel with mocked authorization. Unauthenticated requests to both providers' device-authorization endpoints returned HTTP 200 with the required device fields; no account login or live inference was performed.

## OAuth account usage card

Settings shows an account usage card above Configuration for signed-in Codex OAuth and Kimi providers. The Codex backend can return the plan, used percentages and reset times. Opening the detail page fetches once, and Refresh fetches again. Antigravity usage is supported through dedicated remote quota endpoints.

The backend resolves and refreshes Kivio-owned credentials through the existing OAuth flow, validates the original provider endpoint, then makes a bounded GET with redirects disabled. Only normalized quota fields reach the webview. Transport errors never include tokens or raw response bodies. Missing usage percentages remain unavailable rather than appearing as 0% used.

Protocol references inspected locally on 2026-09-05:

- [Kimi CLI usage implementation](https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/ui/shell/usage.py): `GET https://api.kimi.com/coding/v1/usages`, weekly `usage`, and `limits[].detail` / `limits[].window`. Supports used or remaining counters and absolute/relative reset times.

The existing Kimi CLI Apache-2.0 notice applies to this protocol adaptation. The card displays remaining percentage; these are shared account limits, not Kivio-local token statistics. Tests cover remaining-to-used conversion, missing fields, numeric strings, reset times, manual refresh, failed requests and stale results after disconnect. Verification: 33 frontend tests, three Rust parser tests, TypeScript and changed-file ESLint passed. Live quota responses from an authenticated account have not been verified.

### Antigravity quota follow-up

The same card now supports authorized Antigravity accounts. It calls project-scoped retrieveUserQuotaSummary first, then retrieveUserQuota when grouped data is unavailable. Both use the existing fixed Cloud Code endpoint, account-bound JSON credentials, automatic refresh and redirect-disabled client. It parses grouped quota windows (including nested remaining fields) and model buckets, retains reset timestamps or upstream reset descriptions, and preserves missing/invalid fractions as unknown. It deliberately does not interpret fetchAvailableModels availability as real remaining quota. Remote OAuth may expose fewer windows than the Antigravity app. No local app credentials are imported, and no additional software is launched.

Protocol references: [CodexBar Antigravity notes](https://github.com/steipete/CodexBar/blob/main/docs/antigravity.md), [Antigravity-Manager quota schema](https://github.com/lbjlaq/Antigravity-Manager/blob/main/src-tauri/src/modules/quota.rs), and [Google Gemini CLI quota response example](https://github.com/google-gemini/gemini-cli/issues/14883). Independently implemented schema parsing; no upstream implementation is copied. Live OAuth quota access remains account-dependent and has not been verified in this change.
