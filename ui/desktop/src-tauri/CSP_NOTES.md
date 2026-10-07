# Content-security-policy notes

`tauri.conf.json` is plain JSON and cannot carry comments, so the reasons for
non-obvious `connect-src` origins live here.

- `https://chitin.id`, `https://mainnet.base.org`, `https://arweave.net`: these
  serve the OPTIONAL on-chain identity feature only. They are contacted solely
  after a user fills in Settings, Persona, "On-chain identity (optional)"
  (config keys `identity_chitin_slug` and `identity_chain`, both empty by
  default). With those keys empty no request is made to any of them. The RPC
  origin must also match `CSP_ALLOWED_RPC_ORIGINS` in
  `ui/command-center/src/config/chain.ts`.
